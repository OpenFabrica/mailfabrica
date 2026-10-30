# 原生 Linux 构建容器；依赖库与 Go 工具链均来自公开官方镜像和 Debian 包。
FROM golang:1.26.7-bookworm
RUN apt-get update && apt-get install -y --no-install-recommends \
    libgtk-3-dev libwebkit2gtk-4.1-dev pkg-config python3 file xvfb xauth dbus-x11 \
    && rm -rf /var/lib/apt/lists/*
ENV GOFLAGS=-tags=gtk3 CGO_ENABLED=1
WORKDIR /src
