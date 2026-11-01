package main

import (
	"image"
	"image/draw"
	"math"
)

type scaleFilter struct {
	support float64
	at      func(x float64) float64
}

var (
	catmullRom = scaleFilter{2, func(x float64) float64 {
		x = math.Abs(x)
		if x < 1 {
			return (3*x-5)*x*x/2 + 1
		}
		if x < 2 {
			return ((5-x)*x-8)*x/2 + 2
		}
		return 0
	}}
	bilinear = scaleFilter{1, func(x float64) float64 { return max(0, 1-math.Abs(x)) }}
)

func scaleWeights(m, n int, f scaleFilter) ([]int, [][]float32) {
	ratio := float64(m) / float64(n)
	width := max(ratio, 1)
	reach := f.support * width
	starts := make([]int, n)
	weights := make([][]float32, n)
	for o := range n {
		center := (float64(o)+0.5)*ratio - 0.5
		lo := max(0, int(math.Ceil(center-reach)))
		hi := min(m-1, int(math.Floor(center+reach)))
		if hi < lo {
			lo, hi = min(max(int(center+0.5), 0), m-1), min(max(int(center+0.5), 0), m-1)
		}
		w := make([]float32, hi-lo+1)
		var sum float64
		for i := lo; i <= hi; i++ {
			v := f.at((float64(i) - center) / width)
			w[i-lo] = float32(v)
			sum += v
		}
		if sum == 0 {
			w[len(w)/2] = 1
		} else {
			for i := range w {
				w[i] = float32(float64(w[i]) / sum)
			}
		}
		starts[o], weights[o] = lo, w
	}
	return starts, weights
}

func scaleImage(src image.Image, sr image.Rectangle, w, h int, f scaleFilter) *image.RGBA {
	sw, sh := sr.Dx(), sr.Dy()
	xs, xw := scaleWeights(sw, w, f)
	ys, yw := scaleWeights(sh, h, f)
	row := image.NewRGBA(image.Rect(0, 0, sw, 1))
	mid := make([]float32, w*sh*4)
	for y := range sh {
		draw.Draw(row, row.Bounds(), src, image.Pt(sr.Min.X, sr.Min.Y+y), draw.Src)
		out := mid[y*w*4 : (y+1)*w*4]
		for x := range w {
			var r, g, b, a float32
			p := row.Pix[xs[x]*4:]
			for i, k := range xw[x] {
				r += k * float32(p[i*4])
				g += k * float32(p[i*4+1])
				b += k * float32(p[i*4+2])
				a += k * float32(p[i*4+3])
			}
			out[x*4], out[x*4+1], out[x*4+2], out[x*4+3] = r, g, b, a
		}
	}
	dst := image.NewRGBA(image.Rect(0, 0, w, h))
	for y := range h {
		d := dst.Pix[y*dst.Stride:]
		for x := range w {
			var c [4]float32
			for i, k := range yw[y] {
				p := mid[((ys[y]+i)*w+x)*4:]
				c[0] += k * p[0]
				c[1] += k * p[1]
				c[2] += k * p[2]
				c[3] += k * p[3]
			}
			a := clampByte(c[3], 255)
			d[x*4], d[x*4+1], d[x*4+2], d[x*4+3] = clampByte(c[0], a), clampByte(c[1], a), clampByte(c[2], a), a
		}
	}
	return dst
}

func clampByte(v float32, limit uint8) uint8 {
	if v <= 0 {
		return 0
	}
	if v >= float32(limit) {
		return limit
	}
	return uint8(v + 0.5)
}
