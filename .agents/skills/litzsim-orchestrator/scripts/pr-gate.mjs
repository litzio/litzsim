#!/usr/bin/env node
// Check that a LitzSim PR may be merged into main; optionally record the note and merge it.
// LitzSim has no CI or merge queue, so the note must carry a local verification line
// (from pr-verify.mjs) that names the current PR head and the current origin/main.
// Usage: node pr-gate.mjs --pr <N> [--merge --note-file <file> --ai-agent <agent> --ai-model <provider/model>]
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

const REPO = 'litzio/litzsim';
const BASE = 'main';
const run = (command, args, options = {}) => execFileSync(command, args, { encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'], ...options }).trim();
const gh = args => run('gh', args);
const git = (args, options) => run('git', args, options);

export const MODEL_ID = /^[a-z0-9][a-z0-9._-]*\/[a-z0-9][a-z0-9._/-]*$/;

// Git owns trailer parsing; the source scan only catches duplicated or misplaced AI trailers.
export function trailerProblem(message, cwd = process.cwd()) {
  const normalized = message.replace(/\r\n/g, '\n');
  const parsed = git(['interpret-trailers', '--parse'], { cwd, input: normalized });
  const finalBlock = normalized.trimEnd().split(/\n[\t ]*\n/).at(-1).split('\n');
  for (const key of ['AI-Agent', 'AI-Model']) {
    const mentions = normalized.split('\n').filter(line => new RegExp(`^[\\t ]*${key}[\\t ]*:`, 'i').test(line));
    const entries = parsed.split('\n').filter(line => line.toLowerCase().startsWith(`${key.toLowerCase()}:`));
    if (mentions.length !== 1 || entries.length !== 1) return `${key} must occur exactly once in the final trailer block`;
    const value = entries[0].slice(key.length + 1).trim();
    if (!value || !finalBlock.includes(`${key}: ${value}`)) return `${key} must have a value in the final contiguous trailer block`;
    if (key === 'AI-Model' && !MODEL_ID.test(value)) return 'AI-Model must be a canonical <provider>/<model> id';
  }
  return null;
}

// The record must state the review decision so that skipping a review is always a written judgment.
export const reviewDecision = note => /^독립 리뷰: (APPROVE [0-9a-f]{7,64}|생략 — \S)/.test(note.replace(/^﻿/, '').trimStart());

// Returns { head, main } from the "로컬 검증:" line written by pr-verify.mjs, or null.
export function verificationRecord(note) {
  const line = note.match(/^로컬 검증: head ([0-9a-f]{40}) on main ([0-9a-f]{40}) — (.*)$/m);
  if (!line) return null;
  // A docs-only tree records that nothing was built; otherwise every test must have passed.
  if (!/^빌드 대상 없음/.test(line[3])) {
    const count = line[3].match(/(?:CTest|시험) (\d+)\/(\d+) 통과/);
    if (!count || count[1] !== count[2] || count[1] === '0') return null;
  }
  return { head: line[1], main: line[2] };
}

export function checkProblems(checks) {
  const state = check => check.conclusion || check.state || check.status || '';
  const pending = checks.filter(check => !state(check) || ['PENDING', 'QUEUED', 'IN_PROGRESS', 'EXPECTED'].includes(state(check)));
  const failed = checks.filter(check => !pending.includes(check) && !['SUCCESS', 'SKIPPED', 'NEUTRAL'].includes(state(check)));
  const problems = [];
  if (pending.length) problems.push(`checks pending: ${pending.map(check => check.name ?? check.context).join(', ')}`);
  if (failed.length) problems.push(`checks failed: ${failed.map(check => `${check.name ?? check.context}=${state(check)}`).join(', ')}`);
  return problems;
}

async function main() {
  const argv = process.argv.slice(2);
  const option = name => { const i = argv.indexOf(name); return i >= 0 ? argv[i + 1] : undefined; };
  const pr = Number(option('--pr'));
  const merge = argv.includes('--merge');
  const noteFile = option('--note-file');
  const aiAgent = option('--ai-agent');
  const aiModel = option('--ai-model');
  if (!Number.isInteger(pr) || pr <= 0) throw new Error('Usage: pr-gate.mjs --pr <N> [--merge --note-file <file> --ai-agent <agent> --ai-model <provider/model>]');
  if (merge && (!noteFile || !aiAgent || !aiModel)) throw new Error('--merge requires --note-file, --ai-agent and --ai-model');
  if (merge && !MODEL_ID.test(aiModel)) throw new Error('--ai-model must be a canonical <provider>/<model> id');

  if (!/github\.com[/:]litzio\/litzsim(\.git)?$/.test(git(['remote', 'get-url', 'origin']))) {
    throw new Error('This script only runs inside the litzio/litzsim repository');
  }
  const root = git(['rev-parse', '--show-toplevel']);

  const view = JSON.parse(gh(['pr', 'view', String(pr), '--repo', REPO, '--json',
    'state,isDraft,title,headRefName,headRefOid,baseRefName,mergeable,statusCheckRollup']));
  const problems = [];
  if (view.state !== 'OPEN') problems.push(`PR is ${view.state}`);
  if (view.baseRefName !== BASE) problems.push(`base is ${view.baseRefName}, not ${BASE}`);
  if (view.isDraft) problems.push('PR is a draft (run `gh pr ready` first)');
  if (view.mergeable === 'CONFLICTING') problems.push(`PR conflicts with ${BASE}`);
  problems.push(...checkProblems(view.statusCheckRollup ?? []));

  const threads = JSON.parse(gh(['api', 'graphql', '-f',
    `query={repository(owner:"litzio",name:"litzsim"){pullRequest(number:${pr}){reviewThreads(first:100){nodes{isResolved}}}}}`]))
    .data.repository.pullRequest.reviewThreads.nodes;
  const unresolved = threads.filter(thread => !thread.isResolved).length;
  if (unresolved) problems.push(`${unresolved} unresolved review thread(s)`);

  if (view.state !== 'OPEN') {
    console.log(JSON.stringify({ pr, head: view.headRefOid, problems }, null, 2));
    process.exit(1);
  }

  git(['fetch', '-q', 'origin', BASE, view.headRefName], { cwd: root });
  const main = git(['rev-parse', `refs/remotes/origin/${BASE}`], { cwd: root });
  const fetched = git(['rev-parse', `refs/remotes/origin/${view.headRefName}`], { cwd: root });
  if (fetched !== view.headRefOid) problems.push(`remote branch ${fetched} differs from PR head ${view.headRefOid}`);
  else {
    const base = git(['merge-base', main, view.headRefOid], { cwd: root });
    const commits = git(['rev-list', `${base}..${view.headRefOid}`], { cwd: root }).split('\n').filter(Boolean);
    for (const sha of commits) {
      const problem = trailerProblem(git(['show', '-s', '--format=%B', sha], { cwd: root }), root);
      if (problem) problems.push(`trailer ${sha.slice(0, 10)}: ${problem}`);
    }
  }

  if (noteFile) {
    const note = readFileSync(noteFile, 'utf8');
    if (!reviewDecision(note)) problems.push('note must start with "독립 리뷰: APPROVE <head>" or "독립 리뷰: 생략 — <사유>"');
    const record = verificationRecord(note);
    if (!record) problems.push('note has no passing "로컬 검증: head <sha> on main <sha> — … 시험 n/n 통과" (or "빌드 대상 없음") line (run pr-verify.mjs)');
    else {
      if (record.head !== view.headRefOid) problems.push(`local verification is for head ${record.head.slice(0, 10)}, PR head is ${view.headRefOid.slice(0, 10)} (run pr-verify.mjs again)`);
      if (record.main !== main) problems.push(`local verification is on main ${record.main.slice(0, 10)}, origin/main is ${main.slice(0, 10)} (run pr-verify.mjs again)`);
    }
  } else if (merge) problems.push('missing note file');

  console.log(JSON.stringify({ pr, head: view.headRefOid, main, unresolved, problems }, null, 2));
  if (problems.length) process.exit(1);
  if (!merge) process.exit(0);

  gh(['pr', 'comment', String(pr), '--repo', REPO, '--body-file', noteFile]);
  gh(['pr', 'merge', String(pr), '--repo', REPO, '--merge', '--match-head-commit', view.headRefOid,
    '--body', `${view.title}\n\nAI-Agent: ${aiAgent}\nAI-Model: ${aiModel}`]);
  const after = JSON.parse(gh(['pr', 'view', String(pr), '--repo', REPO, '--json', 'state,mergeCommit']));
  console.log(JSON.stringify({ pr, state: after.state, mergeCommit: after.mergeCommit?.oid ?? null }));
  if (after.state !== 'MERGED') process.exit(1);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) await main();
