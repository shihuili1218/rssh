/** English UI strings. Keep keys stable, sorted by area. */
const en = {
  // ── Common ──
  "common.save": "Save",
  "common.saving": "Saving...",
  "common.cancel": "Cancel",
  "common.delete": "Delete",
  "common.edit": "Edit",
  "common.add": "Add",
  "common.back": "Back",
  "common.close": "Close",
  "common.confirm": "Confirm",
  "common.yes": "Yes",
  "common.no": "No",
  "common.loading": "Loading...",
  "common.connecting": "Connecting...",
  "common.connected": "Connected",
  "common.disconnected": "Disconnected",
  "common.disconnect": "Disconnect",
  "common.reconnect": "Reconnect",
  "common.copy": "Copy",
  "common.paste": "Paste",
  "common.search": "Search",

  // ── Sidebar / Tabs ──
  "tab.home": "Home",
  "tab.local": "Local",
  "tab.new_terminal": "New Terminal",
  "tab.new_edit": "New Edit",
  "tab.settings": "Settings",
  "window.pin": "Pin on Top",
  "tab.context.search": "Search",
  "tab.context.snippets": "Snippets",
  "tab.context.sftp": "SFTP Browser",
  "tab.context.clone": "Clone Tab",
  "tab.context.close": "Close Tab",
  "tab.context.open_new_window": "Open in New Window",

  // ── Settings menu ──
  "settings.title": "Settings",
  "settings.section.profiles": "Profiles",
  "settings.section.credentials": "Credentials",
  "settings.section.forwards": "Port Forwards",
  "settings.section.groups": "Groups",
  "settings.section.snippets": "Snippets",
  "settings.section.highlights": "Keyword Highlights",
  "settings.section.recording": "Session Recording",
  "settings.section.shell": "Shell",
  "settings.section.github_sync": "GitHub Sync",
  "settings.section.import_export": "Import / Export",
  "settings.section.cli": "CLI Tool",
  "settings.section.shortcuts": "Shortcuts",
  "settings.section.about": "About",
  "about.repo": "GitHub Repo",
  "about.issues": "Report an Issue",
  "about.license": "License",
  "about.diagnostics": "Copy Diagnostics",
  "about.diagnostics.hint": "Attach this when reporting an issue",
  "about.copied": "Copied",

  // ── Toast shared ──
  "toast.error.delete": "Delete failed",
  "toast.error.add": "Add failed",
  "toast.error.save": "Save failed",
  "toast.error.reset": "Reset failed",
  "settings.section.language": "Language",

  // ── Language picker ──
  "lang.english": "English",
  "lang.chinese": "中文",

  // ── Home screen ──
  "home.empty.title": "No profiles yet",
  "home.empty.cta": "Create one in Settings → Profiles to get started.",
  "home.connect": "Connect",

  // ── Profile editor ──
  "profile.name": "Name",
  "profile.host": "Host",
  "profile.port": "Port",
  "profile.username": "Username",
  "profile.credential": "Credential",
  "profile.bastion": "Bastion (jump host)",
  "profile.init_command": "Init command",
  "profile.group": "Group",
  "profile.none": "(none)",

  // ── Credential editor ──
  "credential.name": "Name",
  "credential.username": "Username",
  "credential.auth_type": "Auth Type",
  "credential.type.password": "Password",
  "credential.type.key": "Private Key (PEM)",
  "credential.type.agent": "SSH Agent ($SSH_AUTH_SOCK / Pageant)",
  "credential.type.none": "None",
  "credential.type.interactive": "Keyboard Interactive",
  "credential.password": "Password",
  "credential.private_key": "Private Key",
  "credential.passphrase": "Passphrase (optional)",
  "credential.passphrase_placeholder": "Key passphrase",
  "credential.agent_hint": "Authentication will use the keys loaded into your local SSH agent. No secret is stored.",
  "credential.sync_to_remote": "SYNC TO REMOTE",
  "credential.sync_to_remote_desc": "Include this credential's secret when pushing to GitHub.",

  // ── Forward editor ──
  "forward.name": "Name",
  "forward.type": "Type",
  "forward.type.local": "Local (-L)",
  "forward.type.remote": "Remote (-R)",
  "forward.type.dynamic": "Dynamic (-D, SOCKS5)",
  "forward.local_port": "Local Port",
  "forward.remote_host": "Remote Host",
  "forward.remote_port": "Remote Port",
  "forward.profile": "Profile",
  "forward.start": "Start",
  "forward.stop": "Stop",
  "forward.bytes_sent": "Bytes sent",
  "forward.bytes_received": "Bytes received",
  "forward.active_connections": "Active connections",
};

export default en;
export type Messages = typeof en;
export type MessageKey = keyof Messages;
