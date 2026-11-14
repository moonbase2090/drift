# Install

The package is named `omarchy-drift` and provides the `drift` command. It targets Omarchy on x86_64 Arch Linux. Other packages provide and conflict with the bare name `drift`.

## Signed pacman repository

These steps work after the first signed release has been published. Download the public key:

```bash
curl -fLO https://raw.githubusercontent.com/moonbase2090/drift/main/keys/drift-pacman-signing.pub.asc
gpg --show-keys --with-fingerprint drift-pacman-signing.pub.asc
```

Check the full fingerprint against the published value:

```text
F636 5E85 6F3A 896B ABDA A4BD 3787 05A4 14BF 786C
```

The Ed25519 signing key expires on 2028-11-12. Import and locally trust that key:

```bash
sudo pacman-key --add drift-pacman-signing.pub.asc
sudo pacman-key --lsign-key F6365E856F3A896BABDAA4BD378705A414BF786C
```

Add this block to `/etc/pacman.conf`:

```ini
[omarchy-drift]
SigLevel = Required DatabaseRequired
Server = https://github.com/moonbase2090/drift/releases/download/pacman-repo
```

Both the repository database and package require signatures. Install with a full system upgrade:

```bash
sudo pacman -Syu omarchy-drift
drift --version
systemctl --user enable --now drift-notify.path
drift widget install
```

`drift widget install` previews the plugin installation and asks before writing. If the package conflicts with another provider of `drift`, review pacman's proposed replacement before accepting. Run `drift widget sync` after upgrading to refresh the installed bar widget.

To uninstall, remove the widget before the binary (`drift widget remove`), disable the notice (`systemctl --user disable --now drift-notify.path`), then remove the package with `sudo pacman -R omarchy-drift`. Runtime snapshots under `/var/lib/drift` and user backups remain. See the [history and undo instructions](../README.md#history-and-undo) before deleting them.

## AUR

AUR publishing is planned for later, when registration reopens. The package name will remain `omarchy-drift`. No AUR account or SSH key is needed for the signed repository above.
