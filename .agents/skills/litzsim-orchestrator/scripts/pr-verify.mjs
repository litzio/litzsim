#!/usr/bin/env node
// Build and test what main would become if a LitzSim PR were merged now.
// Creates a temporary detached worktree at the PR head, merges the current origin/main into it
// without committing, builds and runs every test, then removes the worktree. A tree with Cargo.toml uses
// cargo (Release build, whole workspace); one with CMakeLists.txt uses CMake/CTest; a docs-only tree records
// that there was nothing to build. On success the last line is the record that pr-gate.mjs requires:
//   로컬 검증: head <sha> on main <sha> — cargo Release 빌드, 시험 n/n 통과 (…)
// Usage: node pr-verify.mjs --pr <N> [--keep]
// For CMake trees the toolchain bin (default C:\msys64\ucrt64\bin when it exists) can be set with LITZSIM_TOOLCHAIN_BIN.
import { execFileSync, spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

const REPO = 'litzio/litzsim';
const BASE = 'main';
const run = (command, args, options = {}) => execFileSync(command, args, { encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'], ...options }).trim();

export function ctestSummary(output) {
  const match = output.match(/(\d+)% tests passed, (\d+) tests? failed out of (\d+)/);
  if (!match) return null;
  const total = Number(match[3]);
  return { passed: total - Number(match[2]), total };
}

// Sums every "test result:" line (unit, integration and doc tests) of `cargo test`.
export function cargoSummary(output) {
  const results = [...output.matchAll(/test result: \w+\. (\d+) passed; (\d+) failed/g)];
  if (!results.length) return null;
  const passed = results.reduce((sum, match) => sum + Number(match[1]), 0);
  const failed = results.reduce((sum, match) => sum + Number(match[2]), 0);
  return { passed, total: passed + failed };
}

function step(label, command, args, options) {
  const started = Date.now();
  console.error(`== ${label}`);
  const result = spawnSync(command, args, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], ...options });
  const seconds = (Date.now() - started) / 1000;
  const output = `${result.stdout ?? ''}${result.stderr ?? ''}`;
  if (result.error || result.status !== 0) {
    console.error(output.split('\n').slice(-60).join('\n'));
    throw new Error(`${label} failed${result.error ? `: ${result.error.message}` : ` (exit ${result.status})`}`);
  }
  console.error(`   ${seconds.toFixed(1)} s`);
  return { output, seconds };
}

async function main() {
  const argv = process.argv.slice(2);
  const i = argv.indexOf('--pr');
  const pr = Number(i >= 0 ? argv[i + 1] : NaN);
  const keep = argv.includes('--keep');
  if (!Number.isInteger(pr) || pr <= 0) throw new Error('Usage: pr-verify.mjs --pr <N> [--keep]');

  if (!/github\.com[/:]litzio\/litzsim(\.git)?$/.test(run('git', ['remote', 'get-url', 'origin']))) {
    throw new Error('This script only runs inside the litzio/litzsim repository');
  }
  const root = run('git', ['rev-parse', '--show-toplevel']);
  const view = JSON.parse(run('gh', ['pr', 'view', String(pr), '--repo', REPO, '--json', 'headRefOid,baseRefName,state']));
  if (view.baseRefName !== BASE) throw new Error(`PR base is ${view.baseRefName}, not ${BASE}`);
  if (view.state !== 'OPEN') console.error(`note: PR is ${view.state}; verifying anyway`);

  run('git', ['fetch', '-q', '--no-tags', 'origin', BASE, `refs/pull/${pr}/head`], { cwd: root });
  const head = view.headRefOid;
  const main = run('git', ['rev-parse', `refs/remotes/origin/${BASE}`], { cwd: root });
  run('git', ['cat-file', '-e', `${head}^{commit}`], { cwd: root });

  const bin = process.env.LITZSIM_TOOLCHAIN_BIN ?? (existsSync('C:\\msys64\\ucrt64\\bin') ? 'C:\\msys64\\ucrt64\\bin' : null);
  const env = { ...process.env, ...(bin ? { PATH: `${bin}${path.delimiter}${process.env.PATH}` } : {}) };

  const dir = mkdtempSync(path.join(tmpdir(), `litzsim-verify-${pr}-`));
  const build = path.join(dir, 'build');
  let added = false;
  try {
    run('git', ['worktree', 'add', '--detach', '-q', dir, head], { cwd: root });
    added = true;
    const ancestor = spawnSync('git', ['merge-base', '--is-ancestor', main, head], { cwd: root }).status === 0;
    if (!ancestor) {
      const merged = spawnSync('git', ['-c', 'user.name=pr-verify', '-c', 'user.email=pr-verify@invalid',
        'merge', '--no-commit', '--no-ff', '-q', main], { cwd: dir, encoding: 'utf8' });
      if (merged.status !== 0) {
        const conflicts = run('git', ['diff', '--name-only', '--diff-filter=U'], { cwd: dir });
        throw new Error(`merge of origin/${BASE} into PR head conflicts: ${conflicts.split('\n').join(', ') || merged.stderr.trim()}`);
      }
    }
    const merged = ancestor ? '' : ', main 병합 결과';
    const prefix = `로컬 검증: head ${head} on main ${main} —`;
    if (existsSync(path.join(dir, 'Cargo.toml'))) {
      const cargoEnv = { ...process.env, CARGO_TARGET_DIR: path.join(dir, 'target') };
      const compile = step('cargo build', 'cargo', ['build', '--release', '--workspace', '--all-targets'], { cwd: dir, env: cargoEnv });
      const test = step('cargo test', 'cargo', ['test', '--release', '--workspace'], { cwd: dir, env: cargoEnv });
      const summary = cargoSummary(test.output);
      if (!summary || summary.passed !== summary.total || summary.total === 0) {
        console.error(test.output.split('\n').slice(-30).join('\n'));
        throw new Error('cargo test summary missing, no tests ran, or not all tests passed');
      }
      console.log(`${prefix} cargo Release 빌드, 시험 ${summary.passed}/${summary.total} 통과 (빌드 ${(compile.seconds / 60).toFixed(1)}분, 시험 ${test.seconds.toFixed(1)} s${merged})`);
    } else if (existsSync(path.join(dir, 'CMakeLists.txt'))) {
      const configure = step('configure', 'cmake', ['-S', dir, '-B', build, '-G', 'Ninja', '-DCMAKE_BUILD_TYPE=Release'], { env });
      const compile = step('build', 'cmake', ['--build', build], { env });
      const test = step('ctest', 'ctest', ['--test-dir', build, '--output-on-failure'], { env });
      const summary = ctestSummary(test.output);
      if (!summary || summary.passed !== summary.total || summary.total === 0) {
        console.error(test.output.split('\n').slice(-30).join('\n'));
        throw new Error('CTest summary missing or not all tests passed');
      }
      const minutes = ((configure.seconds + compile.seconds) / 60).toFixed(1);
      console.log(`${prefix} Release 빌드, CTest ${summary.passed}/${summary.total} 통과 (빌드 ${minutes}분, 시험 ${test.seconds.toFixed(1)} s${merged})`);
    } else {
      // A docs-only tree (the start of the rebuild) has nothing to compile; say so instead of passing silently.
      console.log(`${prefix} 빌드 대상 없음 (Cargo.toml·CMakeLists.txt 없음${merged})`);
    }
  } finally {
    if (added && !keep) {
      spawnSync('git', ['worktree', 'remove', '--force', dir], { cwd: root });
      spawnSync('git', ['worktree', 'prune'], { cwd: root });
    } else if (keep) console.error(`kept ${dir}`);
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try { await main(); } catch (error) { console.error(`FAILED: ${error.message}`); process.exit(1); }
}
