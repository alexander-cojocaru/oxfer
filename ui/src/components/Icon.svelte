<script lang="ts" module>
  // Inline SVG icons (no remote fonts / images; CSP-friendly).
  const paths = {
    folder: 'M3 6.5A1.5 1.5 0 0 1 4.5 5h4l2 2h9A1.5 1.5 0 0 1 21 8.5v9a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 17.5z',
    file: 'M6 3h8l5 5v12a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1zm8 0v5h5',
    link: 'M10 14a4 4 0 0 0 5.66 0l3-3a4 4 0 0 0-5.66-5.66l-1 1M14 10a4 4 0 0 0-5.66 0l-3 3a4 4 0 0 0 5.66 5.66l1-1',
    up: 'M12 19V5m-6 6 6-6 6 6',
    refresh: 'M20 11a8 8 0 1 0-2.34 5.66M20 4v7h-7',
    copy: 'M5 12h12m-5-6 6 6-6 6',
    plus: 'M12 5v14M5 12h14',
    trash: 'M4 7h16M10 11v6m4-6v6M6 7l1 13h10l1-13M9 7V4h6v3',
    edit: 'M4 20h4L19 9l-4-4L4 16zM13 7l4 4',
    info: 'M12 8h.01M11 12h1v5h1M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18z',
    eye: 'M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12zm10 3a3 3 0 1 0 0-6 3 3 0 0 0 0 6z',
    eyeOff: 'M3 3l18 18M10.6 5.1A10.8 10.8 0 0 1 12 5c6.5 0 10 7 10 7a17.6 17.6 0 0 1-3.2 4.2M6.6 6.6C3.9 8.4 2 12 2 12s3.5 7 10 7a9.9 9.9 0 0 0 5.4-1.6M9.9 9.9a3 3 0 0 0 4.2 4.2',
    pause: 'M8 5v14M16 5v14',
    play: 'M7 4v16l13-8z',
    x: 'M6 6l12 12M18 6 6 18',
    retry: 'M4 4v6h6M20 20v-6h-6M5.6 15A8 8 0 0 0 19 16.4M18.4 9A8 8 0 0 0 5 7.6',
    plug: 'M9 2v6m6-6v6M6 8h12v3a6 6 0 0 1-12 0zm6 9v5',
    monitor: 'M3 4h18v12H3zM8 20h8M12 16v4',
    server: 'M4 4h16v6H4zm0 10h16v6H4zM8 7h.01M8 17h.01',
    laptop: 'M4 5h16v11H4zM2 19h20',
    sidebar: 'M4 4h16v16H4zM9 4v16',
    chevronDown: 'M6 9l6 6 6-6',
    chevronRight: 'M9 6l6 6-6 6',
    shield: 'M12 3l8 3v6c0 5-3.5 8-8 9-4.5-1-8-4-8-9V6z',
    key: 'M15 7a3 3 0 1 1-3 3M3 21l7.5-7.5M7 17l2 2M12 10a5 5 0 1 0 5-5',
    warning: 'M12 3 2 21h20zM12 10v5m0 3h.01',
    check: 'M5 12l5 5L20 7',
    terminal: 'M4 5h16v14H4zM7 10l3 2-3 2M12 15h5',
    search: 'M11 18a7 7 0 1 1 0-14 7 7 0 0 1 0 14zm10 3-5-5',
  } as const;
  export type IconName = keyof typeof paths;
  export { paths };
</script>

<script lang="ts">
  let { name, size = 16, label }: { name: IconName; size?: number; label?: string } = $props();
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="1.8"
  stroke-linecap="round"
  stroke-linejoin="round"
  role={label ? 'img' : undefined}
  aria-label={label}
  aria-hidden={label ? undefined : 'true'}
  focusable="false"
>
  <path d={paths[name]} />
</svg>
