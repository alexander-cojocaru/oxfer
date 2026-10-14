# Changelog

All notable changes to oxfer. The newest release comes first. This file is
shown in the desktop app (version number in the status bar).

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
