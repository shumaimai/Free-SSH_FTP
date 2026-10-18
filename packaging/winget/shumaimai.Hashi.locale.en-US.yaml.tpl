# yaml-language-server: $schema=https://aka.ms/winget-manifest.defaultLocale.1.6.0.schema.json
PackageIdentifier: shumaimai.Hashi
PackageVersion: @VERSION@
PackageLocale: en-US
Publisher: shumaimai
PublisherUrl: https://github.com/shumaimai
PublisherSupportUrl: https://github.com/shumaimai/Free-SSH_FTP/issues
PackageName: Hashi
PackageUrl: https://github.com/shumaimai/Free-SSH_FTP
License: MIT
LicenseUrl: https://github.com/shumaimai/Free-SSH_FTP/blob/main/LICENSE
ShortDescription: SSH terminal and SFTP file browser in one tab, with optional AI assistance and local MCP.
Description: |-
  Hashi combines an SSH terminal and an SFTP file browser in one tab (no more juggling PuTTY and WinSCP).
  Dual-pane local/remote browsing, safe overwrite transfers that keep the original until the new file is verified,
  host-key verification, ProxyJump, port forwarding, and an optional AI assistant / local MCP server
  where every action on your terminal goes through an explicit approval step.
  The app has no telemetry; the only network traffic is to your own servers, the AI services you register,
  and an optional update check against GitHub releases.
Moniker: hashi
Tags:
  - sftp
  - ssh
  - terminal
  - tauri
  - rust
  - mcp
ReleaseNotesUrl: https://github.com/shumaimai/Free-SSH_FTP/releases/tag/v@VERSION@
ManifestType: defaultLocale
ManifestVersion: 1.6.0
