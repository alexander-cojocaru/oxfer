import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import VncViewer from './VncViewer.svelte';
import { api } from '../lib/api';

vi.mock('../lib/api', () => ({
  api: {
    vncOpen: vi.fn(),
    vncKey: vi.fn(),
    vncPointer: vi.fn(),
    vncClipboard: vi.fn(),
    vncClose: vi.fn(),
  },
  errorMessage: (e: { message?: string }) => e?.message ?? String(e),
}));

const mocked = vi.mocked(api);
const request = { kind: 'profile' as const, name: 'desk' };
const info = { id: 7, label: 'desk', width: 800, height: 600, name: 'Desktop' };

const rectMessage = new Uint8Array([1, 1, 0, 2, 0, 1, 0, 1, 0, 9, 8, 7, 255]).buffer;
const copyMessage = new Uint8Array([2, 4, 0, 4, 0, 1, 0, 1, 0, 0, 0, 0, 0]).buffer;
const closedMessage = new Uint8Array([5, ...new TextEncoder().encode('server went away')]).buffer;

const ctx = { putImageData: vi.fn(), drawImage: vi.fn() };

beforeEach(() => {
  vi.clearAllMocks();
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(ctx as unknown as CanvasRenderingContext2D);
  vi.stubGlobal(
    'ImageData',
    class {
      constructor(
        public data: Uint8ClampedArray,
        public width: number,
        public height: number,
      ) {}
    },
  );
  vi.stubGlobal(
    'ResizeObserver',
    class {
      observe() {}
      disconnect() {}
    },
  );
  mocked.vncKey.mockResolvedValue(undefined);
  mocked.vncPointer.mockResolvedValue(undefined);
  mocked.vncClose.mockResolvedValue(undefined);
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe('VncViewer', () => {
  it('connects, draws updates that arrive early and later, and sends keys', async () => {
    let deliver: (data: ArrayBuffer) => void = () => {};
    mocked.vncOpen.mockImplementation(async (_req, onMessage) => {
      deliver = onMessage;
      // The server's first update beats the reply of `vncOpen`.
      onMessage(rectMessage);
      return info;
    });
    render(VncViewer, { props: { request, label: 'desk', onclose: vi.fn() } });

    expect(screen.getByRole('status')).toHaveTextContent('Connecting to desk');
    await screen.findByText('800×600');
    expect(mocked.vncOpen).toHaveBeenCalledWith(request, expect.any(Function));
    expect(ctx.putImageData).toHaveBeenCalledTimes(1);
    expect(ctx.putImageData.mock.calls[0].slice(1)).toEqual([1, 2]);

    deliver(copyMessage);
    // copy (4,4) 1x1 from (0,0)
    expect(ctx.drawImage).toHaveBeenCalledWith(expect.anything(), 0, 0, 1, 1, 4, 4, 1, 1);

    const canvas = screen.getByLabelText('Remote screen');
    await fireEvent.keyDown(canvas, { key: 'a', code: 'KeyA' });
    await fireEvent.keyUp(canvas, { key: 'a', code: 'KeyA' });
    await waitFor(() => expect(mocked.vncKey).toHaveBeenCalledTimes(2));
    expect(mocked.vncKey.mock.calls).toEqual([
      [7, 0x61, true],
      [7, 0x61, false],
    ]);
  });

  it('releases a key with the keysym it was pressed with', async () => {
    mocked.vncOpen.mockResolvedValue(info);
    render(VncViewer, { props: { request, label: 'desk', onclose: vi.fn() } });
    await screen.findByText('800×600');
    const canvas = screen.getByLabelText('Remote screen');
    await fireEvent.keyDown(canvas, { key: 'a', code: 'KeyA' });
    // Shift went down meanwhile: the release event reports "A".
    await fireEvent.keyUp(canvas, { key: 'A', code: 'KeyA' });
    await waitFor(() => expect(mocked.vncKey).toHaveBeenCalledTimes(2));
    expect(mocked.vncKey.mock.calls[1]).toEqual([7, 0x61, false]);
  });

  it('shows why the connection ended and reconnects', async () => {
    const user = userEvent.setup();
    let deliver: (data: ArrayBuffer) => void = () => {};
    mocked.vncOpen.mockImplementation(async (_req, onMessage) => {
      deliver = onMessage;
      return info;
    });
    render(VncViewer, { props: { request, label: 'desk', onclose: vi.fn() } });
    await screen.findByText('800×600');

    deliver(closedMessage);
    expect(await screen.findByRole('alert')).toHaveTextContent('server went away');
    // The server side is already gone: no close call is needed.
    await user.click(screen.getByRole('button', { name: 'Reconnect' }));
    expect(mocked.vncOpen).toHaveBeenCalledTimes(2);
    await waitFor(() => expect(screen.queryByRole('alert')).not.toBeInTheDocument());
  });

  it('reports a failed connection', async () => {
    mocked.vncOpen.mockRejectedValue({ code: 'auth', message: 'authentication failed: nope' });
    render(VncViewer, { props: { request, label: 'desk', onclose: vi.fn() } });
    expect(await screen.findByRole('alert')).toHaveTextContent('authentication failed: nope');
    expect(mocked.vncClose).not.toHaveBeenCalled();
  });

  it('closes the screen when the viewer goes away', async () => {
    mocked.vncOpen.mockResolvedValue(info);
    const { unmount } = render(VncViewer, { props: { request, label: 'desk', onclose: vi.fn() } });
    await screen.findByText('800×600');
    unmount();
    expect(mocked.vncClose).toHaveBeenCalledWith(7);
  });

  it('closes a screen that finished opening after the viewer was gone', async () => {
    let finish: (v: typeof info) => void = () => {};
    mocked.vncOpen.mockReturnValue(new Promise((resolve) => (finish = resolve)));
    const { unmount } = render(VncViewer, { props: { request, label: 'desk', onclose: vi.fn() } });
    unmount();
    finish(info);
    await waitFor(() => expect(mocked.vncClose).toHaveBeenCalledWith(7));
  });

  it('the Disconnect button asks to close', async () => {
    const user = userEvent.setup();
    mocked.vncOpen.mockResolvedValue(info);
    const onclose = vi.fn();
    render(VncViewer, { props: { request, label: 'desk', onclose } });
    await screen.findByText('800×600');
    await user.click(screen.getByRole('button', { name: 'Disconnect' }));
    expect(onclose).toHaveBeenCalled();
  });

  it('Ctrl+Alt+Del presses the three keys and releases them in reverse', async () => {
    const user = userEvent.setup();
    mocked.vncOpen.mockResolvedValue(info);
    render(VncViewer, { props: { request, label: 'desk', onclose: vi.fn() } });
    await screen.findByText('800×600');
    await user.click(screen.getByRole('button', { name: 'Ctrl+Alt+Del' }));
    await waitFor(() => expect(mocked.vncKey).toHaveBeenCalledTimes(6));
    expect(mocked.vncKey.mock.calls.map(([, sym, down]) => [sym, down])).toEqual([
      [0xffe3, true],
      [0xffe9, true],
      [0xffff, true],
      [0xffff, false],
      [0xffe9, false],
      [0xffe3, false],
    ]);
  });
});
