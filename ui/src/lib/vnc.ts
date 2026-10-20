// Pure helpers of the remote screen (VNC) viewer: decoding the binary screen
// messages sent by the backend (documented in crates/oxfer-vnc/src/message.rs)
// and turning keyboard and pointer events into what the VNC protocol wants.

export type VncOp =
  | { type: 'resize'; width: number; height: number }
  | { type: 'rect'; x: number; y: number; width: number; height: number; rgba: Uint8ClampedArray<ArrayBuffer> }
  | { type: 'copy'; x: number; y: number; width: number; height: number; srcX: number; srcY: number };

export type VncMessage =
  | { kind: 'update'; ops: VncOp[] }
  | { kind: 'bell' }
  | { kind: 'clipboard'; text: string }
  | { kind: 'closed'; reason: string | null };

const TAG_RESIZE = 0;
const TAG_RECT = 1;
const TAG_COPY = 2;
const TAG_BELL = 3;
const TAG_CLIPBOARD = 4;
const TAG_CLOSED = 5;

const utf8 = new TextDecoder();

/** Decode one message from the backend. Throws on malformed data. */
export function parseVncMessage(data: ArrayBuffer | Uint8Array | number[]): VncMessage {
  const bytes = data instanceof Uint8Array ? data : new Uint8Array(data);
  if (bytes.length === 0) throw new Error('empty screen message');
  const tag = bytes[0];
  if (tag === TAG_BELL) return { kind: 'bell' };
  if (tag === TAG_CLIPBOARD) return { kind: 'clipboard', text: utf8.decode(bytes.subarray(1)) };
  if (tag === TAG_CLOSED) {
    const reason = utf8.decode(bytes.subarray(1));
    return { kind: 'closed', reason: reason === '' ? null : reason };
  }

  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const ops: VncOp[] = [];
  let o = 0;
  const u16 = () => {
    if (o + 2 > bytes.length) throw new Error('truncated screen message');
    const v = view.getUint16(o, true);
    o += 2;
    return v;
  };
  while (o < bytes.length) {
    const t = bytes[o++];
    if (t === TAG_RESIZE) {
      ops.push({ type: 'resize', width: u16(), height: u16() });
    } else if (t === TAG_RECT) {
      const x = u16();
      const y = u16();
      const width = u16();
      const height = u16();
      const len = width * height * 4;
      if (o + len > bytes.length) throw new Error('truncated screen message');
      ops.push({ type: 'rect', x, y, width, height, rgba: new Uint8ClampedArray(bytes.buffer as ArrayBuffer, bytes.byteOffset + o, len) });
      o += len;
    } else if (t === TAG_COPY) {
      const x = u16();
      const y = u16();
      const width = u16();
      const height = u16();
      const srcX = u16();
      const srcY = u16();
      ops.push({ type: 'copy', x, y, width, height, srcX, srcY });
    } else {
      throw new Error(`unknown screen record ${t}`);
    }
  }
  return { kind: 'update', ops };
}

// X11 keysyms of the keys that have no character.
const SPECIAL_KEYS: Record<string, number> = {
  Backspace: 0xff08,
  Tab: 0xff09,
  Enter: 0xff0d,
  Pause: 0xff13,
  ScrollLock: 0xff14,
  Escape: 0xff1b,
  Home: 0xff50,
  ArrowLeft: 0xff51,
  ArrowUp: 0xff52,
  ArrowRight: 0xff53,
  ArrowDown: 0xff54,
  PageUp: 0xff55,
  PageDown: 0xff56,
  End: 0xff57,
  PrintScreen: 0xff61,
  Insert: 0xff63,
  ContextMenu: 0xff67,
  NumLock: 0xff7f,
  CapsLock: 0xffe5,
  Delete: 0xffff,
};

// Modifiers are told apart by `code`: `key` does not say left from right.
const MODIFIER_CODES: Record<string, number> = {
  ShiftLeft: 0xffe1,
  ShiftRight: 0xffe2,
  ControlLeft: 0xffe3,
  ControlRight: 0xffe4,
  // Command on a Mac keyboard. Meta rather than Super: Apple's server maps it to Command.
  MetaLeft: 0xffe7,
  MetaRight: 0xffe8,
  AltLeft: 0xffe9,
  AltRight: 0xffea,
};

export interface KeyLike {
  key: string;
  code: string;
}

/** The X11 keysym for a keyboard event, or `null` if it has none (dead keys, IME). */
export function keysymFor(e: KeyLike): number | null {
  const mod = MODIFIER_CODES[e.code];
  if (mod !== undefined) return mod;
  const special = SPECIAL_KEYS[e.key];
  if (special !== undefined) return special;
  const f = /^F(\d{1,2})$/.exec(e.key);
  if (f && +f[1] >= 1 && +f[1] <= 24) return 0xffbd + +f[1];
  // A single character (an astral one is two UTF-16 units but one code point).
  const chars = [...e.key];
  if (chars.length === 1) {
    const cp = chars[0].codePointAt(0)!;
    if (cp < 0x20 || (cp >= 0x7f && cp < 0xa0)) return null;
    return cp < 0x100 ? cp : 0x01000000 + cp;
  }
  return null;
}

/** Keysyms for Ctrl+Alt+Delete, as a press followed by the release in reverse. */
export const CTRL_ALT_DEL = [0xffe3, 0xffe9, 0xffff];

/** RFB button mask for `MouseEvent.buttons` (JS: 1 left, 2 right, 4 middle). */
export function buttonMask(buttons: number): number {
  return (buttons & 1) | ((buttons & 4) >> 1) | ((buttons & 2) << 1);
}

/** RFB button mask bit for a wheel movement, or 0 for none. Wheel up = 8, down = 16, left = 32, right = 64. */
export function wheelMask(deltaX: number, deltaY: number): number {
  if (Math.abs(deltaY) >= Math.abs(deltaX)) return deltaY < 0 ? 8 : deltaY > 0 ? 16 : 0;
  return deltaX < 0 ? 32 : 64;
}

/** Screen coordinates for a pointer position over the canvas, clamped to it. */
export function screenPoint(
  clientX: number,
  clientY: number,
  box: { left: number; top: number; width: number; height: number },
  screen: { width: number; height: number },
): { x: number; y: number } {
  if (box.width <= 0 || box.height <= 0) return { x: 0, y: 0 };
  const clamp = (v: number, max: number) => Math.max(0, Math.min(max - 1, Math.floor(v)));
  return {
    x: clamp(((clientX - box.left) / box.width) * screen.width, screen.width),
    y: clamp(((clientY - box.top) / box.height) * screen.height, screen.height),
  };
}

/** Display scale: fit the screen into the available space, or 1:1. */
export function viewScale(
  mode: 'fit' | 'actual',
  screen: { width: number; height: number },
  area: { width: number; height: number },
): number {
  if (mode === 'actual' || screen.width <= 0 || screen.height <= 0 || area.width <= 0 || area.height <= 0) return 1;
  return Math.min(area.width / screen.width, area.height / screen.height);
}
