// listOffsetIndex 二分定位指定滚动位置所属的邮件行，日期标题已包含在该行高度中。
export function listOffsetIndex(offsets: number[], position: number) {
  let low = 0,
    high = Math.max(0, offsets.length - 1);
  while (low < high) {
    const middle = Math.floor((low + high + 1) / 2);
    if (offsets[middle] <= position) low = middle;
    else high = middle - 1;
  }
  return low;
}

// listWindow 只保留当前一屏和合计一屏缓冲；开头向下预载，途中上下各保留半屏。
export function listWindow(offsets: number[], top: number, height: number) {
  const count = Math.max(0, offsets.length - 1);
  const total = offsets[count] || 0;
  if (!count || height <= 0)
    return { start: 0, end: 0, before: 0, after: total };
  const scrollTop = Math.max(0, Math.min(top, Math.max(0, total - height)));
  const from = Math.max(0, scrollTop - height / 2);
  const to = Math.min(total, from + height * 2);
  const start = Math.min(count, listOffsetIndex(offsets, from));
  const atEnd = listOffsetIndex(offsets, to);
  const end = Math.min(count, atEnd + (offsets[atEnd] < to ? 1 : 0));
  return {
    start,
    end,
    before: offsets[start] || 0,
    after: total - (offsets[end] || 0),
  };
}

// listScrollAnchor 在收件、删除或密度变化后保留首个可见邮件的位置；锚点删除时选择后续仍存在的行。
export function listScrollAnchor(
  oldKeys: string[],
  oldOffsets: number[],
  keys: string[],
  offsets: number[],
  top: number,
) {
  const positions = new Map(keys.map((key, index) => [key, index]));
  const first = listOffsetIndex(oldOffsets, top);
  for (let index = first; index < oldKeys.length; index++) {
    const next = positions.get(oldKeys[index]);
    if (next !== undefined)
      return offsets[next] + (index === first ? top - oldOffsets[index] : 0);
  }
  return top;
}
