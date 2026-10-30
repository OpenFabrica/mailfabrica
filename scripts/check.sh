#!/bin/sh
set -eu
# 质量检查只使用 Go 和前端工具链，禁止悄悄回退到 Wails 预览版。
test "$(go list -m -f '{{.Version}}' github.com/wailsapp/wails/v3)" = "v3.0.0"
test -z "$(gofmt -l cmd internal frontend/assets.go)"
npx --prefix frontend prettier --check 'frontend/src/*.{ts,tsx,css,json}' frontend/package.json frontend/tsconfig.json frontend/vite.config.ts
go vet ./...
go test ./...
go test -race ./internal/... ./cmd/mailfabrica-cli
