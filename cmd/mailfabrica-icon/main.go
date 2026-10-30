// mailfabrica-icon 从代码生成构建图标，仓库不存放位图素材。
package main

import (
	"flag"
	"fmt"
	"image"
	"image/color"
	"image/png"
	"math"
	"os"
	"path/filepath"
)

// segmentDistance 计算像素到图标线段的距离。
func segmentDistance(x, y, ax, ay, bx, by float64) float64 {
	dx, dy := bx-ax, by-ay
	t := ((x-ax)*dx + (y-ay)*dy) / (dx*dx + dy*dy)
	t = math.Max(0, math.Min(1, t))
	return math.Hypot(x-ax-t*dx, y-ay-t*dy)
}

// icon 以超采样绘制自有信封标志，避免依赖厂商素材。
func icon(size int) *image.NRGBA {
	out := image.NewNRGBA(image.Rect(0, 0, size, size))
	lines := [][4]float64{{.23, .32, .77, .32}, {.77, .32, .77, .70}, {.77, .70, .23, .70}, {.23, .70, .23, .32}, {.23, .34, .50, .54}, {.50, .54, .77, .34}}
	for y := 0; y < size; y++ {
		for x := 0; x < size; x++ {
			var red, green, blue, alpha float64
			for sy := 0; sy < 3; sy++ {
				for sx := 0; sx < 3; sx++ {
					fx := (float64(x) + (float64(sx)+.5)/3) / float64(size)
					fy := (float64(y) + (float64(sy)+.5)/3) / float64(size)
					dx := math.Max(math.Abs(fx-.5)-.29, 0)
					dy := math.Max(math.Abs(fy-.5)-.29, 0)
					if dx*dx+dy*dy > .16*.16 {
						continue
					}
					r, g, b := 79., 70., 229.
					for _, line := range lines {
						if segmentDistance(fx, fy, line[0], line[1], line[2], line[3]) < .018 {
							r, g, b = 255, 255, 255
							break
						}
					}
					red += r
					green += g
					blue += b
					alpha++
				}
			}
			if alpha > 0 {
				out.SetNRGBA(x, y, color.NRGBA{R: uint8(red / alpha), G: uint8(green / alpha), B: uint8(blue / alpha), A: uint8(alpha / 9 * 255)})
			}
		}
	}
	return out
}

// main 生成 macOS iconset 所需的全部尺寸。
func main() {
	out := flag.String("out", "dist/MailFabrica.iconset", "图标输出目录")
	flag.Parse()
	if e := os.MkdirAll(*out, 0755); e != nil {
		fmt.Fprintln(os.Stderr, e)
		os.Exit(1)
	}
	for _, size := range []int{16, 32, 128, 256, 512} {
		for scale := 1; scale <= 2; scale++ {
			name := fmt.Sprintf("icon_%dx%d", size, size)
			if scale == 2 {
				name += "@2x"
			}
			file, e := os.Create(filepath.Join(*out, name+".png"))
			if e != nil {
				fmt.Fprintln(os.Stderr, e)
				os.Exit(1)
			}
			e = png.Encode(file, icon(size*scale))
			closeErr := file.Close()
			if e != nil || closeErr != nil {
				fmt.Fprintln(os.Stderr, "无法保存图标")
				os.Exit(1)
			}
		}
	}
}
