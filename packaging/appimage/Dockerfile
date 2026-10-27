# syntax=docker/dockerfile:1
#
# Toolchain for building the Linux AppImage.
#
# The AppImage has to run on as many distributions as possible, so it is linked
# against an old, widely deployed glibc instead of Nix's bleeding-edge nixpkgs.
# Debian 11 (bullseye) ships glibc 2.31, which covers Ubuntu 20.04+, Debian 11+
# and every newer release. SabiRuby's source compiler is built with the C
# toolchain below; no system Ruby installation is needed.
#
# This image only carries the toolchain. The actual build runs through
# tools/build-appimage.sh with the repository bind-mounted at /src, which keeps
# the Rust target directory cacheable between CI runs (see the workflow).

FROM debian:bullseye AS toolchain

ARG RUST_VERSION=1.95.0
ARG APPIMAGETOOL_URL=https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage

ENV DEBIAN_FRONTEND=noninteractive \
    RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    LIBCLANG_PATH=/usr/lib/llvm-11/lib \
    PATH=/usr/local/cargo/bin:$PATH

# Native dependencies mirror default.nix's runtimeLibraries (udev, ALSA, X11,
# xkbcommon, Wayland, Vulkan) plus the pieces the build scripts need: libclang
# for native dependencies, and patchelf to make the staged libraries relocatable.
#
# Debian 11 is end-of-life, so its packages now live on archive.debian.org. An
# EOL base is deliberate: the whole point is to link against an old glibc, and
# the image only provides a build toolchain, it is never shipped to users.
RUN printf 'deb http://archive.debian.org/debian bullseye main\ndeb http://archive.debian.org/debian-security bullseye-security main\n' \
      > /etc/apt/sources.list \
    && printf 'Acquire::Check-Valid-Until "false";\n' \
      > /etc/apt/apt.conf.d/99no-check-valid-until \
    && apt-get update && apt-get install -y --no-install-recommends \
      build-essential \
      ca-certificates \
      clang \
      curl \
      file \
      libasound2-dev \
      libclang-dev \
      libudev-dev \
      libvulkan-dev \
      libwayland-dev \
      libx11-dev \
      libxcb1-dev \
      libxcb-render0-dev \
      libxcb-shape0-dev \
      libxcb-xfixes0-dev \
      libxcursor-dev \
      libxi-dev \
      libxinerama-dev \
      libxkbcommon-dev \
      libxkbcommon-x11-dev \
      libxrandr-dev \
      patchelf \
      pkg-config \
    && rm -rf /var/lib/apt/lists/*

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
      | sh -s -- -y --profile minimal --default-toolchain "${RUST_VERSION}"

# The Ruby profiler needs stackprof. It installs into Ruby's gem path, which
# tools/package-runtime.py bundles like any other gem.
RUN gem install stackprof -v 0.2.28 --no-document

# Unpack appimagetool so it runs without FUSE, which containers rarely provide.
RUN mkdir -p /opt/appimagetool \
    && curl -fsSL "${APPIMAGETOOL_URL}" -o /opt/appimagetool/appimagetool.AppImage \
    && chmod +x /opt/appimagetool/appimagetool.AppImage \
    && cd /opt/appimagetool \
    && ./appimagetool.AppImage --appimage-extract \
    && rm appimagetool.AppImage \
    && mv squashfs-root unpacked

# Allow the build to run as an arbitrary uid (CI passes `--user`), while still
# letting it write to the Rust, Ruby and gem installs and read appimagetool.
RUN chmod -R a+rwX /usr/local/cargo /usr/local/rustup /opt/appimagetool

WORKDIR /src
