import assert from "node:assert/strict";
import test from "node:test";
import {
  listOffsetIndex,
  listScrollAnchor,
  listWindow,
} from "./mail-list-window.ts";

// 验证一万行只挂载一屏和一屏缓冲，同时保留完整滚动条高度。
test("one visible screen and one prefetched screen", () => {
  const offsets = Array.from({ length: 10001 }, (_, i) => i * 48);
  const first = listWindow(offsets, 0, 480);
  assert.deepEqual(first, { start: 0, end: 20, before: 0, after: 479040 });
  const middle = listWindow(offsets, 48000, 480);
  assert.equal(middle.end - middle.start, 20);
  assert.ok(offsets[middle.start] <= 48000);
  assert.ok(offsets[middle.end] >= 48480);
  assert.equal(
    middle.before +
      (offsets[middle.end] - offsets[middle.start]) +
      middle.after,
    480000,
  );
  const bottom = listWindow(offsets, 480000, 480);
  assert.equal(bottom.end, 10000);
});

// 验证窗口高度变化、日期标题边界与未测量、空列表不会漏行或一次渲染全部。
test("resize, dates and empty lists", () => {
  assert.equal(listWindow([0, 48, 96], 0, 0).end, 0);
  assert.deepEqual(listWindow([0], 100, 480), {
    start: 0,
    end: 0,
    before: 0,
    after: 0,
  });
  assert.equal(listOffsetIndex([0, 70, 118, 188], 70), 1);
  const range = listWindow([0, 70, 118, 188], 70, 48);
  assert.ok(range.start <= 1 && range.end > 1);
  const offsets = Array.from({ length: 101 }, (_, i) => i * 94);
  assert.ok(listWindow(offsets, 0, 800).end > listWindow(offsets, 0, 400).end);
  assert.equal(listWindow(offsets, 0, 400).end, 9);
});

// 验证前方新增、删除和可见锚点被删除时保持可预测位置，不受未挂载行影响。
test("anchor survives arrivals and deletions", () => {
  assert.equal(
    listScrollAnchor(
      ["a", "b", "c"],
      [0, 48, 96, 144],
      ["new", "a", "b", "c"],
      [0, 48, 96, 144, 192],
      55,
    ),
    103,
  );
  assert.equal(
    listScrollAnchor(
      ["a", "b", "c"],
      [0, 48, 96, 144],
      ["b", "c"],
      [0, 48, 96],
      55,
    ),
    7,
  );
  assert.equal(
    listScrollAnchor(
      ["a", "b", "c"],
      [0, 48, 96, 144],
      ["a", "c"],
      [0, 48, 96],
      55,
    ),
    48,
  );
});
