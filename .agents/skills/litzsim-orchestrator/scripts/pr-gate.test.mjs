import assert from 'node:assert/strict';
import test from 'node:test';
import * as gate from './pr-gate.mjs';
import { cargoSummary, ctestSummary } from './pr-verify.mjs';

const head = 'a'.repeat(40);
const main = 'b'.repeat(40);

test('accepts exactly one canonical AI trailer pair in the final block', () => {
  assert.equal(gate.trailerProblem('제목\n\n본문\n\nAI-Agent: Codex\nAI-Model: openai/gpt-6.2-sol\n'), null);
  assert.match(gate.trailerProblem('제목\n\nAI-Agent: Codex\n'), /AI-Model/);
  assert.match(gate.trailerProblem('제목\n\nAI-Agent: Codex\nAI-Model: GPT-6\n'), /canonical/);
  assert.match(gate.trailerProblem('제목\n\nAI-Agent: A\nAI-Agent: B\nAI-Model: openai/x\n'), /exactly once/);
  assert.match(gate.trailerProblem('제목\n\nAI-Agent: Codex\n\n본문\n\nAI-Model: openai/x\n'), /AI-Agent/);
});

test('requires a written review decision on the first line', () => {
  assert.ok(gate.reviewDecision(`﻿독립 리뷰: APPROVE ${head}\n`));
  assert.ok(gate.reviewDecision('독립 리뷰: 생략 — 문서만 바뀜\n'));
  assert.ok(!gate.reviewDecision('작업자: x\n독립 리뷰: 생략 — 문서만 바뀜\n'));
});

test('reads only a fully passing local verification line', () => {
  const line = `로컬 검증: head ${head} on main ${main} — Release 빌드, CTest 9/9 통과 (빌드 3.0분, 시험 20.0 s)`;
  assert.deepEqual(gate.verificationRecord(`독립 리뷰: 생략 — x\n${line}\n`), { head, main });
  assert.equal(gate.verificationRecord(line.replace('9/9', '8/9')), null);
  assert.equal(gate.verificationRecord('로컬 검증: 통과'), null);
  const cargo = `로컬 검증: head ${head} on main ${main} — cargo Release 빌드, 시험 42/42 통과 (빌드 1.0분, 시험 3.0 s)`;
  assert.deepEqual(gate.verificationRecord(cargo), { head, main });
  assert.equal(gate.verificationRecord(cargo.replace('42/42', '41/42')), null);
  assert.deepEqual(gate.verificationRecord(`로컬 검증: head ${head} on main ${main} — 빌드 대상 없음 (Cargo.toml·CMakeLists.txt 없음)`), { head, main });
});

test('blocks pending or failing GitHub checks but allows none', () => {
  assert.deepEqual(gate.checkProblems([]), []);
  assert.equal(gate.checkProblems([{ name: 'build', conclusion: 'FAILURE' }]).length, 1);
  assert.equal(gate.checkProblems([{ name: 'build', status: 'IN_PROGRESS' }]).length, 1);
});

test('sums every cargo test result line', () => {
  const output = 'test result: ok. 3 passed; 0 failed; 0 ignored\ntest result: ok. 2 passed; 0 failed; 0 ignored\n';
  assert.deepEqual(cargoSummary(output), { passed: 5, total: 5 });
  assert.deepEqual(cargoSummary('test result: FAILED. 4 passed; 1 failed; 0 ignored'), { passed: 4, total: 5 });
  assert.equal(cargoSummary('Compiling litzsim'), null);
});

test('parses the CTest summary', () => {
  assert.deepEqual(ctestSummary('100% tests passed, 0 tests failed out of 9\n'), { passed: 9, total: 9 });
  assert.deepEqual(ctestSummary('89% tests passed, 1 tests failed out of 9\n'), { passed: 8, total: 9 });
  assert.equal(ctestSummary('No tests were found!!!'), null);
});
