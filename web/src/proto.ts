// Palm 二进制指针帧构造（§4.3，little-endian）。一帧 ≤ 7 字节。

const MOVE = 0x01;
const SCROLL = 0x02;
const BUTTON = 0x03;
const TAP = 0x04;
const PINCH = 0x05;
const GESTURE = 0x06;

export const GESTURE_ID = { UP: 0, DOWN: 1, LEFT: 2, RIGHT: 3 } as const;
export const BTN = { LEFT: 0, RIGHT: 1, MIDDLE: 2 } as const;

const clamp16 = (v: number) => Math.max(-32768, Math.min(32767, Math.round(v)));

function frame(bytes: number[]): Uint8Array {
  return new Uint8Array(bytes);
}

function putI16(bytes: number[], off: number, v: number) {
  const x = clamp16(v);
  bytes[off] = x & 0xff;
  bytes[off + 1] = (x >> 8) & 0xff;
}

export function moveFrame(dx: number, dy: number): Uint8Array {
  const b = [MOVE, 0, 0, 0, 0];
  putI16(b, 1, dx);
  putI16(b, 3, dy);
  return frame(b);
}

export function scrollFrame(dx: number, dy: number): Uint8Array {
  const b = [SCROLL, 0, 0, 0, 0];
  putI16(b, 1, dx);
  putI16(b, 3, dy);
  return frame(b);
}

export function buttonFrame(btn: number, down: boolean): Uint8Array {
  return frame([BUTTON, btn, down ? 1 : 0]);
}

export function tapFrame(btn: number, count: number): Uint8Array {
  return frame([TAP, btn, count]);
}

export function pinchFrame(dscale: number, dx: number, dy: number): Uint8Array {
  const b = [PINCH, 0, 0, 0, 0, 0, 0];
  putI16(b, 1, dscale);
  putI16(b, 3, dx);
  putI16(b, 5, dy);
  return frame(b);
}

export function gestureFrame(id: number): Uint8Array {
  return frame([GESTURE, id]);
}
