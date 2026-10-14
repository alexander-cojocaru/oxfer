import { describe, expect, it } from 'vitest';
import { buttonMask, keysymFor, parseVncMessage, screenPoint, viewScale, wheelMask } from './vnc';

const enc = new TextEncoder();

describe('parseVncMessage', () => {
  it('decodes updates made of several records', () => {
    const bytes = new Uint8Array([
      0, 2, 1, 3, 0, // resize 258x3
      1, 1, 0, 2, 0, 1, 0, 1, 0, 9, 8, 7, 255, // 1x1 rect at (1,2)
      2, 1, 0, 2, 0, 3, 0, 4, 0, 5, 0, 6, 0, // copy
    ]);
    const msg = parseVncMessage(bytes.buffer);
    expect(msg.kind).toBe('update');
    if (msg.kind !== 'update') return;
    expect(msg.ops).toHaveLength(3);
    expect(msg.ops[0]).toEqual({ type: 'resize', width: 258, height: 3 });
    const rect = msg.ops[1];
    expect(rect.type).toBe('rect');
    if (rect.type === 'rect') {
      expect([rect.x, rect.y, rect.width, rect.height]).toEqual([1, 2, 1, 1]);
      expect([...rect.rgba]).toEqual([9, 8, 7, 255]);
    }
    expect(msg.ops[2]).toEqual({ type: 'copy', x: 1, y: 2, width: 3, height: 4, srcX: 5, srcY: 6 });
  });

  it('reads rectangle data at an offset inside a larger buffer', () => {
    const outer = new Uint8Array([99, 99, 1, 0, 0, 0, 0, 1, 0, 1, 0, 1, 2, 3, 255]);
    const msg = parseVncMessage(outer.subarray(2));
    expect(msg.kind === 'update' && msg.ops[0].type === 'rect' && [...msg.ops[0].rgba]).toEqual([1, 2, 3, 255]);
  });

  it('decodes control messages', () => {
    expect(parseVncMessage(new Uint8Array([3]))).toEqual({ kind: 'bell' });
    expect(parseVncMessage(new Uint8Array([4, ...enc.encode('héllo')]))).toEqual({ kind: 'clipboard', text: 'héllo' });
    expect(parseVncMessage(new Uint8Array([5]))).toEqual({ kind: 'closed', reason: null });
    expect(parseVncMessage(new Uint8Array([5, ...enc.encode('gone')]))).toEqual({ kind: 'closed', reason: 'gone' });
    expect(parseVncMessage([3])).toEqual({ kind: 'bell' });
  });

  it('rejects malformed messages', () => {
    expect(() => parseVncMessage(new Uint8Array([]))).toThrow();
    expect(() => parseVncMessage(new Uint8Array([9]))).toThrow(/unknown/);
    expect(() => parseVncMessage(new Uint8Array([1, 0, 0, 0, 0, 2, 0, 1, 0, 1, 2, 3]))).toThrow(/truncated/);
    expect(() => parseVncMessage(new Uint8Array([0, 1]))).toThrow(/truncated/);
  });
});

describe('keysymFor', () => {
  const key = (k: string, code = '') => keysymFor({ key: k, code });

  it('maps characters to their code point', () => {
    expect(key('a', 'KeyA')).toBe(0x61);
    expect(key('A', 'KeyA')).toBe(0x41);
    expect(key(' ')).toBe(0x20);
    expect(key('é')).toBe(0xe9);
    expect(key('€')).toBe(0x01000000 + 0x20ac);
    expect(key('😀')).toBe(0x01000000 + 0x1f600);
  });

  it('maps keys without a character', () => {
    expect(key('Enter')).toBe(0xff0d);
    expect(key('Escape')).toBe(0xff1b);
    expect(key('ArrowLeft')).toBe(0xff51);
    expect(key('Delete')).toBe(0xffff);
    expect(key('F1')).toBe(0xffbe);
    expect(key('F12')).toBe(0xffc9);
  });

  it('tells left and right modifiers apart by code', () => {
    expect(key('Shift', 'ShiftLeft')).toBe(0xffe1);
    expect(key('Shift', 'ShiftRight')).toBe(0xffe2);
    expect(key('Control', 'ControlLeft')).toBe(0xffe3);
    expect(key('Alt', 'AltRight')).toBe(0xffea);
    expect(key('Meta', 'MetaLeft')).toBe(0xffe7);
  });

  it('ignores keys it cannot express', () => {
    expect(key('Dead')).toBeNull();
    expect(key('Unidentified')).toBeNull();
    expect(key('Process')).toBeNull();
    expect(key('\u0001')).toBeNull();
  });
});

describe('pointer helpers', () => {
  it('reorders mouse buttons for RFB', () => {
    expect(buttonMask(0)).toBe(0);
    expect(buttonMask(1)).toBe(1); // left
    expect(buttonMask(4)).toBe(2); // middle
    expect(buttonMask(2)).toBe(4); // right
    expect(buttonMask(7)).toBe(7);
  });

  it('turns wheel movement into button bits', () => {
    expect(wheelMask(0, -10)).toBe(8);
    expect(wheelMask(0, 10)).toBe(16);
    expect(wheelMask(-5, 1)).toBe(32);
    expect(wheelMask(5, 1)).toBe(64);
    expect(wheelMask(0, 0)).toBe(0);
  });

  it('maps client coordinates onto the screen', () => {
    const box = { left: 100, top: 50, width: 400, height: 200 };
    const screen = { width: 800, height: 400 };
    expect(screenPoint(100, 50, box, screen)).toEqual({ x: 0, y: 0 });
    expect(screenPoint(300, 150, box, screen)).toEqual({ x: 400, y: 200 });
    expect(screenPoint(10_000, -10_000, box, screen)).toEqual({ x: 799, y: 0 });
    expect(screenPoint(1, 1, { ...box, width: 0 }, screen)).toEqual({ x: 0, y: 0 });
  });

  it('scales to fit or 1:1', () => {
    const screen = { width: 1000, height: 500 };
    expect(viewScale('fit', screen, { width: 500, height: 500 })).toBe(0.5);
    expect(viewScale('fit', screen, { width: 2000, height: 500 })).toBe(1);
    expect(viewScale('actual', screen, { width: 10, height: 10 })).toBe(1);
    expect(viewScale('fit', screen, { width: 0, height: 0 })).toBe(1);
  });
});
