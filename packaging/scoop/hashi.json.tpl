{
    "version": "@VERSION@",
    "description": "SSH terminal and SFTP file browser in one tab, with optional AI assistance and local MCP.",
    "homepage": "https://github.com/shumaimai/Free-SSH_FTP",
    "license": "MIT",
    "url": "https://github.com/shumaimai/Free-SSH_FTP/releases/download/v@VERSION@/Hashi-v@VERSION@-windows-x64.zip",
    "hash": "@SHA256@",
    "extract_dir": "Hashi-v@VERSION@",
    "bin": "Hashi.exe",
    "shortcuts": [
        [
            "Hashi.exe",
            "Hashi"
        ]
    ],
    "checkver": {
        "github": "https://github.com/shumaimai/Free-SSH_FTP"
    },
    "autoupdate": {
        "url": "https://github.com/shumaimai/Free-SSH_FTP/releases/download/v$version/Hashi-v$version-windows-x64.zip",
        "extract_dir": "Hashi-v$version",
        "hash": {
            "url": "$baseurl/SHA256SUMS.txt"
        }
    }
}
