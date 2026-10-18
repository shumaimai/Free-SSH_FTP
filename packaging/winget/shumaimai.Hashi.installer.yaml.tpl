# yaml-language-server: $schema=https://aka.ms/winget-manifest.installer.1.6.0.schema.json
PackageIdentifier: shumaimai.Hashi
PackageVersion: @VERSION@
InstallerType: zip
NestedInstallerType: portable
NestedInstallerFiles:
  - RelativeFilePath: Hashi-v@VERSION@\Hashi.exe
    PortableCommandAlias: hashi
MinimumOSVersion: 10.0.17763.0
Installers:
  - Architecture: x64
    InstallerUrl: https://github.com/shumaimai/Free-SSH_FTP/releases/download/v@VERSION@/Hashi-v@VERSION@-windows-x64.zip
    InstallerSha256: @SHA256@
ManifestType: installer
ManifestVersion: 1.6.0
