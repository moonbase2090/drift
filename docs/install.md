# Install

The package name is `omarchy-drift`. It provides the `drift` command. Other packages already provide and conflict with the bare name `drift`, so this package is not named `drift`.

## Pacman repository

Add this block to `/etc/pacman.conf`:

```
[omarchy-drift]
Server = https://github.com/moonbase2090/drift/releases/download/pacman-repo
SigLevel = Required DatabaseRequired
```

The repository database and the package are signed. The public key is [`keys/drift-pacman-signing.pub.asc`](../keys/drift-pacman-signing.pub.asc). Trust it once, then install:

```
sudo pacman-key --add keys/drift-pacman-signing.pub.asc
sudo pacman-key --lsign-key F6365E856F3A896BABDAA4BD378705A414BF786C
sudo pacman -Syu omarchy-drift
```

A version tag replaces the assets on the `pacman-repo` release. The signing key is the Actions secret `DRIFT_PACMAN_SIGNING_KEY`. The key id is the Actions secret `DRIFT_PACMAN_SIGNING_KEY_ID`. Neither secret is committed. A publish without the signing secret stops before it uploads anything.

## AUR

AUR registration is paused. When it reopens, the package name there is `omarchy-drift`.
