// Package frontend 将前端生产构建嵌入桌面二进制。
package frontend

import (
	"embed"
	"io/fs"
)

//go:embed all:dist
var assets embed.FS

// Assets 返回无额外路径前缀的生产资源。
func Assets() (fs.FS, error) { return fs.Sub(assets, "dist") }
