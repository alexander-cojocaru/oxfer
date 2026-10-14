<script lang="ts">
  import Modal from './Modal.svelte';
  import changelogSource from '../../../CHANGELOG.md?raw';
  import { parseChangelog } from '../lib/changelog';
  import { t } from '../lib/i18n';

  let { onclose }: { onclose: () => void } = $props();

  const releases = parseChangelog(changelogSource);
</script>

<Modal title={t('changelog')} {onclose} width={560}>
  {#if releases.length === 0}
    <p class="muted">{t('changelogEmpty')}</p>
  {/if}
  {#each releases as r (r.version)}
    <section>
      <h3>
        {r.version}
        {#if r.date}<span class="date">{r.date}</span>{/if}
      </h3>
      <ul>
        {#each r.items as item (item)}
          <li>{item}</li>
        {/each}
      </ul>
    </section>
  {/each}
  {#snippet footer()}
    <button type="button" class="btn primary" data-autofocus onclick={onclose}>{t('close')}</button>
  {/snippet}
</Modal>

<style>
  section + section {
    margin-top: 16px;
  }
  h3 {
    margin: 0 0 4px;
    font-size: 13px;
    font-weight: 600;
  }
  .date {
    margin-left: 8px;
    font-weight: 400;
    color: var(--fg-muted);
  }
  ul {
    margin: 0;
    padding-left: 18px;
  }
  li {
    margin: 2px 0;
    user-select: text;
    -webkit-user-select: text;
  }
  .muted {
    color: var(--fg-muted);
  }
</style>
