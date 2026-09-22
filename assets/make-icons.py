#!/usr/bin/env python3
"""生成 CopyPath 的应用图标母图与菜单栏模板小图。

母图按 Apple HIG 的 macOS 图标栅格画：1024 画布、图形区 824×824 居中（四周各留
100pt 呼吸）、圆角半径 185、外部全透明、不烤系统阴影（系统会自己加）。

托盘图必须是纯黑字形 + 透明背景，配合 TrayIconBuilder::icon_as_template(true)
才能在深浅色菜单栏下都由系统正确反色。
"""
from PIL import Image, ImageDraw

SIZE = 1024
ART = 824                      # 图形区边长
MARGIN = (SIZE - ART) // 2     # 100
RADIUS = 185                   # 824 * 22.4%

TOP = (61, 139, 253)
BOTTOM = (29, 101, 216)
WHITE = (255, 255, 255, 255)
FLAP = (232, 241, 253, 255)
INK = (29, 101, 216, 255)

img = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
d = ImageDraw.Draw(img)

# 圆角方形底 + 竖向渐变
mask = Image.new("L", (SIZE, SIZE), 0)
ImageDraw.Draw(mask).rounded_rectangle(
    [MARGIN, MARGIN, SIZE - MARGIN, SIZE - MARGIN], radius=RADIUS, fill=255
)
grad = Image.new("RGBA", (SIZE, SIZE))
gd = ImageDraw.Draw(grad)
for y in range(MARGIN, SIZE - MARGIN):
    t = (y - MARGIN) / (ART - 1)
    gd.line([(0, y), (SIZE, y)], fill=(*[round(a + (b - a) * t) for a, b in zip(TOP, BOTTOM)], 255))
img.paste(grad, (0, 0), mask)

# 文件夹：背板带左上标签页，前板略偏蓝以露出折痕
d.rounded_rectangle([216, 216, 470, 300], radius=34, fill=WHITE)
d.rounded_rectangle([216, 262, 808, 648], radius=44, fill=WHITE)
d.rounded_rectangle([216, 318, 808, 648], radius=44, fill=FLAP)

# 路径胶囊：三段名字块 + 两个分隔箭头
PILL = [273 - 97, 690, 751 + 97, 806]
d.rounded_rectangle(PILL, radius=(PILL[3] - PILL[1]) // 2, fill=WHITE)
DASH_W, GAP, CHEV, DASH_H = 110, 24, 26, 44
top = (PILL[1] + PILL[3]) // 2 - DASH_H // 2
x = PILL[0] + 97
for i in range(3):
    d.rounded_rectangle([x, top, x + DASH_W, top + DASH_H], radius=14, fill=INK)
    x += DASH_W
    if i < 2:
        x += GAP
        y_mid = top + DASH_H // 2
        d.line(
            [(x, top + 6), (x + CHEV, y_mid), (x, top + DASH_H - 6)],
            fill=INK, width=13, joint="curve",
        )
        x += CHEV + GAP

img.save("assets/icon-source.png")
img.resize((512, 512), Image.LANCZOS).save("assets/icon-512.png")

# --- 托盘模板图：36x36 @2x，纯黑 + alpha ---
T = 36
tray = Image.new("RGBA", (T, T), (0, 0, 0, 0))
td = ImageDraw.Draw(tray)
td.rounded_rectangle([6, 10, 30, 26], radius=3, outline=(0, 0, 0, 255), width=2)
td.rounded_rectangle([4, 5, 15, 10], radius=2, outline=(0, 0, 0, 255), width=2)
td.line([(11, 31), (25, 31)], fill=(0, 0, 0, 255), width=2)
tray.save("assets/tray-template@2x.png")

print("wrote assets/icon-source.png, assets/icon-512.png, assets/tray-template@2x.png")
