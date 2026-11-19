# Self-hosted GitHub Actions runner that can build BenNovel (Tauri) for Linux and Android.
# Base: myoung34/github-runner (Ubuntu 22.04, same as the ubuntu-22.04 GitHub runner,
# so the AppImage/deb links against the same glibc as before).
FROM myoung34/github-runner:ubuntu-jammy

ARG DEBIAN_FRONTEND=noninteractive

# Tauri Linux build deps + tools the workflow uses (curl, unzip, file, patchelf, Java for Android).
RUN apt-get update && apt-get install -y --no-install-recommends \
      build-essential pkg-config libssl-dev curl wget file unzip zip xz-utils ca-certificates \
      libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev libxdo-dev patchelf xdg-utils \
      openjdk-17-jdk-headless \
    && rm -rf /var/lib/apt/lists/*

# linuxdeploy (used to make the AppImage) is itself an AppImage and needs FUSE,
# which a container does not have: let it unpack itself instead.
ENV APPIMAGE_EXTRACT_AND_RUN=1

# Rust (stable) with the Android targets the workflow builds.
ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH
RUN curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable \
    && rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android \
    && chmod -R a+w "$RUSTUP_HOME" "$CARGO_HOME"

# Android SDK + NDK. NDK_VERSION must match the one in .github/workflows/release.yml.
ARG ANDROID_CMDLINE_TOOLS=11076708
ARG NDK_VERSION=27.0.12077973
ENV ANDROID_HOME=/opt/android-sdk \
    ANDROID_SDK_ROOT=/opt/android-sdk \
    JAVA_HOME=/usr/lib/jvm/java-17-openjdk-amd64
ENV PATH=$ANDROID_HOME/cmdline-tools/latest/bin:$ANDROID_HOME/platform-tools:$PATH
RUN mkdir -p "$ANDROID_HOME/cmdline-tools" \
    && curl -fsSLo /tmp/clt.zip "https://dl.google.com/android/repository/commandlinetools-linux-${ANDROID_CMDLINE_TOOLS}_latest.zip" \
    && unzip -q /tmp/clt.zip -d "$ANDROID_HOME/cmdline-tools" \
    && mv "$ANDROID_HOME/cmdline-tools/cmdline-tools" "$ANDROID_HOME/cmdline-tools/latest" \
    && rm /tmp/clt.zip \
    && (yes || true) | sdkmanager --licenses > /dev/null \
    && sdkmanager "platform-tools" "platforms;android-35" "build-tools;35.0.0" "ndk;${NDK_VERSION}" > /dev/null \
    && chmod -R a+w "$ANDROID_HOME"
# Gradle downloads any other platform it needs by itself (licenses are accepted above).
