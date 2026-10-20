# Changelog

All notable changes to oxfer. The newest release comes first. This file is
shown in the desktop app (version number in the status bar).

## 0.7.0 - 2026-10-20

- VNC support: a new "VNC" protocol in the connection dialog (VNC and Apple
  Remote Desktop authentication). Saved VNC profiles and quick connect open a
  full-window remote screen viewer with keyboard, mouse, wheel and clipboard,
  scale-to-fit and automatic reconnect.
- Press Esc to close the terminal window.

## 0.6.4 - 2026-10-20

- Fixed the update dialog offering the version you already have: the app now
  reports its real version. Builds before 0.6.4 still report 0.6.0, so they
  will offer this update once.

## 0.6.3 - 2026-10-19

- Release pipeline fix (Homebrew formula step); no changes to the app itself.

## 0.6.2 - 2026-10-19

- Release pipeline fix (Homebrew formula step); no changes to the app itself.

## 0.6.1 - 2026-10-19

- Build and release pipeline now runs on self-hosted runners; no changes to the
  app itself.

## 0.6.0 - 2026-10-14

- Duplicate a profile: right-click a profile and choose "Duplicate profile…".
  Everything is copied (including a stored password); change the name and host
  before saving.
- Export and import profiles: the "⋯" menu in the sidebar (or
  `oxfer profile export` / `oxfer profile import`) writes your profiles and
  credentials to a file and merges them back in. Passwords are never exported.

## 0.5.0 - 2026-10-14

- The app can update itself: click "Check for updates" in the status bar to
  download and install the latest release (macOS).

## 0.4.1 - 2026-10-14

- The changelog can be viewed in the app (click the version in the status bar).
- Releases include a signed macOS installer (`.dmg`) for Apple silicon and Intel.

## 0.4.0 - 2026-10-14

- Quick connect moved to the header.
- Sidebar sections are collapsible and scrollable.

## 0.3.1 - 2026-10-13

- Fixed a crash when closing a terminal.

## 0.3.0 - 2026-10-13

- Polished contrast, interaction states, empty state and dialogs.
- Left click on a profile connects; right click opens the actions menu.
- All secrets are kept in a single keychain item.
- The macOS app is signed with a stable identity so permissions persist.

## 0.2.0 - 2026-10-08

- Open an SSH terminal on a connected server.
- Resizable sidebar, file panes and transfer queue.
- Move bookmarks between groups; sub groups for profiles.
- Text preview of files, copy to clipboard and editing in the text editor.
- Mouse back/forward buttons navigate folder history.
- Full profile name shown in a tooltip on hover.
- One connection per server.
- Bookmarks: groups, search and shared credentials.
