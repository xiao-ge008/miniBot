// 直接加载共享 onPayload 实现，验证监听时序与卸载，避免只测试代码副本。
const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const { test } = require('node:test');
const ts = require('typescript');
const text = fs.readFileSync(path.join(__dirname, '../src/lib/core/ipc.ts'), 'utf8');
const source = text.slice(text.indexOf('export async function onPayload('), text.indexOf('export function onLlmStatus'));
const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText;
const flush = () => new Promise(setImmediate);
function harness(takePayload) {
  const exported = {};
  let handler;
  let stopped = false;
  const errors = [];
  const listen = async (_event, cb) => { handler = cb; return () => { stopped = true; }; };
  const ipc = { takePayload: () => takePayload({ listening: !!handler }) };
  new Function('exports', 'ipc', 'listen', 'console', compiled)(exported, ipc, listen, { error: (...args) => errors.push(args) });
  return { subscribe: exported.onPayload, emit: () => handler(), stopped: () => stopped, errors };
}

test('首次读取前已订阅事件，后续投递不会落在监听空档', async () => {
  const pending = [{ id: 'first' }];
  const h = harness(async ({ listening }) => { assert.ok(listening); return pending.shift() ?? null; });
  const received = [];
  const stop = await h.subscribe(p => { received.push(p.id); });
  await flush();
  pending.push({ id: 'next' });
  h.emit();
  await flush();
  assert.deepEqual(received, ['first', 'next']);
  stop();
  assert.ok(h.stopped());
});

test('卸载后返回的旧负载不能再调用组件回调', async () => {
  let resolve;
  const h = harness(() => new Promise(r => { resolve = r; }));
  const received = [];
  const stop = await h.subscribe(p => { received.push(p); });
  stop();
  resolve({ id: 'late' });
  await flush();
  assert.deepEqual(received, []);
});

test('异步负载处理失败被捕获，不产生未处理拒绝且保留监听', async () => {
  const pending = [{ id: 'first' }];
  const h = harness(async () => pending.shift() ?? null);
  let count = 0;
  const stop = await h.subscribe(async () => { count++; throw new Error('测试异常'); });
  await flush();
  pending.push({ id: 'next' });
  h.emit();
  await flush();
  assert.equal(count, 2);
  assert.equal(h.errors.length, 2);
  stop();
});
