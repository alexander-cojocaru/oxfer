import { fireEvent, render, screen } from '@testing-library/svelte';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import PromptDialog from './PromptDialog.svelte';

describe('PromptDialog', () => {
  it('host key dialog shows the fingerprint and answers', async () => {
    const onanswer = vi.fn();
    render(PromptDialog, {
      props: {
        id: 7,
        prompt: { kind: 'hostKey', host: 'example.com', port: 22, algorithm: 'ssh-ed25519', fingerprint: 'SHA256:abc' },
        onanswer,
      },
    });
    expect(screen.getByRole('alertdialog', { name: 'Unknown host key' })).toBeInTheDocument();
    expect(screen.getByTestId('fingerprint')).toHaveTextContent('SHA256:abc');
    await fireEvent.click(screen.getByRole('button', { name: 'Accept & save' }));
    expect(onanswer).toHaveBeenCalledWith(7, { kind: 'hostKey', decision: 'acceptAndSave' });
  });

  it('escape rejects an unknown host key', async () => {
    const onanswer = vi.fn();
    render(PromptDialog, {
      props: { id: 1, prompt: { kind: 'hostKey', host: 'h', port: 22, algorithm: 'a', fingerprint: 'f' }, onanswer },
    });
    await fireEvent.keyDown(screen.getByRole('alertdialog'), { key: 'Escape' });
    expect(onanswer).toHaveBeenCalledWith(1, { kind: 'hostKey', decision: 'reject' });
  });

  it('password prompt submits the secret', async () => {
    const user = userEvent.setup();
    const onanswer = vi.fn();
    render(PromptDialog, {
      props: { id: 2, prompt: { kind: 'password', user: 'bob', host: 'h', secret: 'password', retry: true }, onanswer },
    });
    expect(screen.getByRole('alert')).toHaveTextContent(/try again/i);
    await user.type(screen.getByLabelText('Password for bob@h'), 'hunter2{Enter}');
    expect(onanswer).toHaveBeenCalledWith(2, { kind: 'password', value: 'hunter2' });
  });

  it('conflict prompt with apply to all', async () => {
    const user = userEvent.setup();
    const onanswer = vi.fn();
    const entry = { name: 'a.txt', path: '/d/a.txt', kind: 'file' as const, size: 3, modified: 0, permissions: null };
    render(PromptDialog, {
      props: { id: 3, prompt: { kind: 'conflict', transferId: 1, src: entry, dst: entry }, onanswer },
    });
    await user.click(screen.getByLabelText('Apply to all remaining conflicts'));
    await user.click(screen.getByRole('button', { name: 'Overwrite' }));
    expect(onanswer).toHaveBeenCalledWith(3, { kind: 'conflict', action: 'overwrite', applyToAll: true });
  });
});
