# Install

The package name is `omarchy-drift`. It provides the `drift` command. Other packages already provide and conflict with the bare name `drift`, so this package is not named `drift`.

## Pacman repository

Add this block to `/etc/pacman.conf`:

```
[omarchy-drift]
Server = https://github.com/moonbase2090/drift/releases/download/pacman-repo
SigLevel = Required DatabaseRequired
```

The repository database and the package are signed. The fingerprint is the `FINGERPRINT` file on that release. It is published when the signing key is configured. Trust it once, then install:

```
sudo pacman-key --recv-keys <fingerprint>
sudo pacman-key --lsign-key <fingerprint>
sudo pacman -Syu omarchy-drift
```

A version tag replaces the assets on the `pacman-repo` release. The signing key is the Actions secret `DRIFT_PACKAGE_KEY`. It is not committed. A publish without that secret stops before it uploads anything.

## AUR

AUR registration is paused. When it reopens, the package name there is `omarchy-drift`.
