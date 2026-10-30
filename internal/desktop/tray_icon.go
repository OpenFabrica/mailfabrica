package desktop

import (
	"bytes"
	"image"
	"image/color"
	"image/draw"
	"image/png"
	"strconv"
)

// trayIcon 用代码绘制自有信封；macOS 使用模板图标，其他系统将未读数绘入托盘图标。
func trayIcon(unread int, template bool) ([]byte, error) {
	const size = 32
	img := image.NewNRGBA(image.Rect(0, 0, size, size))
	ink := color.NRGBA{R: 255, G: 255, B: 255, A: 255}
	if template {
		ink = color.NRGBA{A: 255}
	} else {
		draw.Draw(img, img.Bounds(), image.NewUniform(color.NRGBA{R: 79, G: 70, B: 229, A: 255}), image.Point{}, draw.Src)
	}
	line := func(ax, ay, bx, by int) {
		dx, dy := bx-ax, by-ay
		steps := max(abs(dx), abs(dy))
		for i := 0; i <= steps; i++ {
			x, y := ax+dx*i/max(1, steps), ay+dy*i/max(1, steps)
			draw.Draw(img, image.Rect(x, y, x+2, y+2), image.NewUniform(ink), image.Point{}, draw.Src)
		}
	}
	line(5, 8, 25, 8)
	line(25, 8, 25, 23)
	line(25, 23, 5, 23)
	line(5, 23, 5, 8)
	line(5, 9, 15, 17)
	line(15, 17, 25, 9)
	if !template && unread > 0 {
		text := strconv.Itoa(unread)
		if unread > 99 {
			text = "99+"
		}
		draw.Draw(img, image.Rect(14, 0, 32, 15), image.NewUniform(color.NRGBA{R: 224, G: 52, B: 61, A: 255}), image.Point{}, draw.Src)
		// 3×5 点阵保留小图标的数字辨识度，不引入位图或字体文件。
		glyphs := map[rune][5]string{'0': {"111", "101", "101", "101", "111"}, '1': {"010", "110", "010", "010", "111"}, '2': {"111", "001", "111", "100", "111"}, '3': {"111", "001", "111", "001", "111"}, '4': {"101", "101", "111", "001", "001"}, '5': {"111", "100", "111", "001", "111"}, '6': {"111", "100", "111", "101", "111"}, '7': {"111", "001", "010", "010", "010"}, '8': {"111", "101", "111", "101", "111"}, '9': {"111", "101", "111", "001", "111"}, '+': {"000", "010", "111", "010", "000"}}
		start := 14 + (18-(len(text)*4-1))/2
		for i, char := range text {
			for y, row := range glyphs[char] {
				for x, pixel := range row {
					if pixel == '1' {
						img.SetNRGBA(start+i*4+x, 4+y, color.NRGBA{R: 255, G: 255, B: 255, A: 255})
					}
				}
			}
		}
	}
	var out bytes.Buffer
	if err := png.Encode(&out, img); err != nil {
		return nil, err
	}
	return out.Bytes(), nil
}

// abs 为小图标的像素线段计算整数距离。
func abs(value int) int {
	if value < 0 {
		return -value
	}
	return value
}
