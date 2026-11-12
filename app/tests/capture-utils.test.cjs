// 使用项目现有 TypeScript 编译器加载纯函数测试，不新增测试依赖。
const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const { test } = require('node:test');
const ts = require('typescript');
const source = fs.readFileSync(path.join(__dirname, '../src/lib/core/capture-utils.ts'), 'utf8');
const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText;
const loaded = { exports: {} };
new Function('exports', 'module', compiled)(loaded.exports, loaded);
const { parseCaptureMeta, toCapturePoint, matchesCaptureReset } = loaded.exports;
const valid = () => ({ session_id: 'current', mode: 'chat', frame: { x: -1440, y: 0, width: 2880, height: 1800, scale: 2 }, windows: [[0, 0, 200, 100]], cursor: { x: 300, y: 400 } });

test('两个方向独立换算，兼容非等比视口与混合缩放', () => {
  assert.deepEqual(toCapturePoint(340, 290, 1920, 1080, 720, 620), { x: 907, y: 505 });
  assert.deepEqual(toCapturePoint(-20, 900, 1920, 1080, 720, 620), { x: 0, y: 1080 });
});
test('拒绝缺失会话、空图像、非法尺寸', () => {
  for (const value of [null, {}, { ...valid(), session_id: '' }, { ...valid(), frame: { ...valid().frame, width: 0 } }, { ...valid(), frame: { ...valid().frame, height: NaN } }]) {
    assert.equal(parseCaptureMeta(value), null);
  }
});
test('保留负屏幕原点，并过滤无效窗口和光标值', () => {
  const value = valid();
  value.windows.push([NaN, 0, 200, 100], [0, 0, -1, 100]);
  value.cursor = { x: -30, y: 9000 };
  const parsed = parseCaptureMeta(value);
  assert.equal(parsed.frame.x, -1440);
  assert.equal(parsed.mode, 'chat');
  assert.equal(parsed.windows.length, 1);
  assert.deepEqual(parsed.cursor, { x: 0, y: 1800 });
});
test('旧重置事件不能清空当前截图', () => {
  assert.equal(matchesCaptureReset('new', { session_id: 'old' }), false);
  assert.equal(matchesCaptureReset(null, { session_id: 'new' }), false);
  assert.equal(matchesCaptureReset('new', null), false);
  assert.equal(matchesCaptureReset('new', { session_id: 'new' }), true);
});
