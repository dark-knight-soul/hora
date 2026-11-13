"""
Генератор шрифта «WallClock Split» — широкий геометрический шрифт одной толщины
с фирменными разрывами в буквах (в духе Anurati), латиница + кириллица + цифры.

Каждая буква описана осевыми линиями (штрихами) со скруглениями углов.
Скрипт превращает штрихи в контуры заданной толщины, обрезает наклонные концы
по линии букв и собирает настоящий .ttf через fontTools.

Запуск:  python tools/make_font.py   →   assets/fonts/WallClockSplit.ttf
Нужна библиотека fontTools:  python -m pip install --user fonttools
"""
import math
import os

from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen

# ---------------- основные размеры (единиц на кегль — 1000) ----------------
UPM = 1000
CAP = 700          # высота заглавных
T = 88             # толщина штриха
H = T / 2
G = 96             # ширина «разрыва» (заметный, как у Anurati)
SB = 52            # поля слева и справа от буквы
TOP = CAP - H      # ось верхней горизонтали
BOT = H            # ось нижней горизонтали
MID = CAP / 2      # ось средней горизонтали
R = 1000           # скругление «чаш» (по оси): максимум, что позволяет буква — полукруги, как у Anurati
RR = 1000          # скругление круглых букв: O — почти окружность
BAND = (0, CAP)    # по умолчанию всё обрезается по линии букв
DESC = -150        # нижний вынос (Д, Ц, Щ)


class S:
    """Штрих: точки оси (x, y) или (x, y, радиус скругления), замкнутый или нет."""

    def __init__(self, pts, closed=False, band=BAND, t=T):
        self.pts = [p if len(p) == 3 else (p[0], p[1], 0) for p in pts]
        self.closed = closed
        self.band = band
        self.t = t


# ---------------- геометрия: скругления, обводка, обрезка ----------------

def norm(x, y):
    l = math.hypot(x, y)
    return (x / l, y / l) if l > 1e-9 else (0.0, 0.0)


def fillet(pts, closed):
    """Заменяет углы с радиусом на дуги (точки по оси)."""
    out = []
    n = len(pts)
    for i in range(n):
        x, y, r = pts[i]
        if r <= 0 or (not closed and (i == 0 or i == n - 1)):
            out.append((x, y))
            continue
        ax, ay, _ = pts[i - 1]
        bx, by, _ = pts[(i + 1) % n]
        ux, uy = norm(ax - x, ay - y)
        wx, wy = norm(bx - x, by - y)
        cosphi = max(-1.0, min(1.0, ux * wx + uy * wy))
        phi = math.acos(cosphi)
        if phi < 1e-3 or phi > math.pi - 1e-3:
            out.append((x, y))
            continue
        d = r / math.tan(phi / 2)
        # не длиннее половины соседних отрезков
        d = min(d, math.hypot(ax - x, ay - y) / 2, math.hypot(bx - x, by - y) / 2)
        rr = d * math.tan(phi / 2)
        sx, sy = x + ux * d, y + uy * d
        ex, ey = x + wx * d, y + wy * d
        bxn, byn = norm(ux + wx, uy + wy)
        cx, cy = x + bxn * rr / math.sin(phi / 2), y + byn * rr / math.sin(phi / 2)
        a0 = math.atan2(sy - cy, sx - cx)
        a1 = math.atan2(ey - cy, ex - cx)
        da = a1 - a0
        while da > math.pi:
            da -= 2 * math.pi
        while da < -math.pi:
            da += 2 * math.pi
        steps = max(4, int(abs(da) / math.radians(5)))
        for k in range(steps + 1):
            a = a0 + da * k / steps
            out.append((cx + rr * math.cos(a), cy + rr * math.sin(a)))
    return out


def dedupe(pts, closed):
    """Убирает точки, слипшиеся с соседними (иначе на отрезке нулевой длины обводка ломается)."""
    out = []
    for p in pts:
        if not out or math.hypot(p[0] - out[-1][0], p[1] - out[-1][1]) > 0.75:
            out.append(p)
    if closed and len(out) > 2 and math.hypot(out[0][0] - out[-1][0], out[0][1] - out[-1][1]) <= 0.75:
        out.pop()
    return out


def offset(pts, closed, h):
    """Левая и правая стороны обводки (острые углы — со «срезом» по пределу)."""
    pts = dedupe(pts, closed)
    n = len(pts)
    segs = n if closed else n - 1
    normals = []
    for i in range(segs):
        x0, y0 = pts[i]
        x1, y1 = pts[(i + 1) % n]
        dx, dy = norm(x1 - x0, y1 - y0)
        normals.append((-dy, dx))
    left, right = [], []
    for i in range(n):
        if not closed and i == 0:
            nx, ny = normals[0]
            scale = h
        elif not closed and i == n - 1:
            nx, ny = normals[-1]
            scale = h
        else:
            n1 = normals[i - 1]
            n2 = normals[i % segs]
            mx, my = norm(n1[0] + n2[0], n1[1] + n2[1])
            dot = mx * n1[0] + my * n1[1]
            scale = h / dot if dot > 0.25 else h * 4  # предел остроты угла
            nx, ny = mx, my
        x, y = pts[i]
        left.append((x + nx * scale, y + ny * scale))
        right.append((x - nx * scale, y - ny * scale))
    return left, right


def clip(poly, lo, hi):
    """Обрезка многоугольника горизонтальной полосой lo <= y <= hi."""
    def cut(points, keep, ycut):
        res = []
        m = len(points)
        for i in range(m):
            p, q = points[i], points[(i + 1) % m]
            kp, kq = keep(p), keep(q)
            if kp:
                res.append(p)
            if kp != kq:
                t = (ycut - p[1]) / (q[1] - p[1])
                res.append((p[0] + (q[0] - p[0]) * t, ycut))
        return res
    poly = cut(poly, lambda p: p[1] >= lo, lo)
    if poly:
        poly = cut(poly, lambda p: p[1] <= hi, hi)
    return poly


def area(poly):
    return sum(poly[i][0] * poly[(i + 1) % len(poly)][1] - poly[(i + 1) % len(poly)][0] * poly[i][1] for i in range(len(poly))) / 2


def stroke_contours(s):
    """Штрих → контуры (наружные по часовой стрелке, дырки — против)."""
    pts = fillet(s.pts, s.closed)
    left, right = offset(pts, s.closed, s.t / 2)
    if s.closed:
        a, b = clip(left, *s.band), clip(right, *s.band)
        res = []
        if a and b:
            outer, inner = (a, b) if abs(area(a)) > abs(area(b)) else (b, a)
            if area(outer) > 0:
                outer = outer[::-1]
            if area(inner) < 0:
                inner = inner[::-1]
            res = [outer, inner]
        return res
    poly = clip(left + right[::-1], *s.band)
    if len(poly) < 3:
        return []
    if area(poly) > 0:
        poly = poly[::-1]
    return [poly]


# ---------------- буквы ----------------
# Каждая буква: (ширина рисунка без полей, [штрихи]). Оси: x слева направо, y снизу вверх.

def stem(x, lo=-T, hi=CAP + T):
    return S([(x, lo), (x, hi)])


def hbar(x0, x1, y):
    return S([(x0, y), (x1, y)])


def dot(x, y, band=(-1000, 2000)):
    return S([(x, y), (x, y + T)], band=band)


G_ = {}


def glyph(chars, w, strokes):
    for ch in chars:
        G_[ch] = (w, strokes)


def build_letters():
    # A — «Λ» без перекладины, верх срезан по линии букв
    w = 760
    glyph("AА", w, [S([(H + 20, -80), (w / 2, CAP + 80), (w - H - 20, -80)])])
    # B
    w = 690
    xl, xr = H, w - H
    glyph("BВ", w, [stem(xl), S([(xl, TOP), (xr - 30, TOP, R), (xr - 30, MID, R), (xl, MID)]),
                     S([(xl, MID), (xr, MID, R), (xr, BOT, R), (xl, BOT)])])
    # C
    w = 700
    xl, xr = H, w - H
    glyph("CС", w, [S([(xr + 10, TOP), (xl, TOP, RR), (xl, BOT, RR), (xr + 10, BOT)])])
    # D — ножка отделена от «чаши»
    w = 720
    xl, xr = H, w - H
    x0 = T + G
    glyph("D", w, [stem(xl), S([(x0, TOP), (xr, TOP, RR), (xr, BOT, RR), (x0, BOT)])])
    # E — средняя перекладина отделена от ножки
    w = 620
    xl, xr = H, w - H
    glyph("EЕ", w, [stem(xl), hbar(xl, w, TOP), hbar(xl, w, BOT), hbar(T + G, w - 40, MID)])
    glyph("Ё", w, [stem(xl), hbar(xl, w, TOP), hbar(xl, w, BOT), hbar(T + G, w - 40, MID), dot(w * 0.3, 770), dot(w * 0.72, 770)])
    # F
    glyph("F", w, [stem(xl), hbar(xl, w, TOP), hbar(T + G, w - 40, MID)])
    # G
    w = 720
    xl, xr = H, w - H
    glyph("G", w, [S([(xr + 10, TOP), (xl, TOP, RR), (xl, BOT, RR), (xr, BOT, R), (xr, MID), (w / 2 + 20, MID)])])
    # H — перекладина не доходит до правой ножки
    w = 700
    xl, xr = H, w - H
    glyph("HН", w, [stem(xl), stem(xr), hbar(xl, xr - H - G, MID)])
    # I
    glyph("I", T, [stem(H)])
    # J
    w = 600
    xl, xr = H, w - H
    glyph("J", w, [S([(xr, CAP + T), (xr, BOT, RR), (xl, BOT, R), (xl, 230)])])
    # K — «уголок» отделён от ножки
    w = 700
    xl, xr = H, w - H
    glyph("KК", w, [stem(xl), S([(w + 30, CAP + 90), (T + G + 20, MID), (w + 30, -90)])])
    # L
    w = 600
    xl, xr = H, w - H
    glyph("L", w, [S([(xl, CAP + T), (xl, BOT), (w, BOT)])])
    # M
    w = 880
    xl, xr = H, w - H
    glyph("MМ", w, [S([(xl, -T), (xl, TOP), (w / 2, MID - 60), (xr, TOP), (xr, -T)])])
    # N
    w = 720
    xl, xr = H, w - H
    glyph("N", w, [S([(xl, -T), (xl, TOP), (xr, BOT), (xr, CAP + T)])])
    # O
    w = 740
    xl, xr = H, w - H
    glyph("OО", w, [S([(xl, BOT, RR), (xr, BOT, RR), (xr, TOP, RR), (xl, TOP, RR)], closed=True)])
    # P
    w = 670
    xl, xr = H, w - H
    glyph("PР", w, [stem(xl), S([(xl, TOP), (xr, TOP, R), (xr, MID - 20, R), (xl, MID - 20)])])
    # Q
    w = 740
    xl, xr = H, w - H
    glyph("Q", w, [S([(xl, BOT, RR), (xr, BOT, RR), (xr, TOP, RR), (xl, TOP, RR)], closed=True),
                   S([(w / 2 + 70, 190), (w + 40, -90)])])
    # R — ножка отделена
    w = 690
    xl, xr = H, w - H
    glyph("R", w, [stem(xl), S([(xl, TOP), (xr, TOP, R), (xr, MID - 20, R), (xl, MID - 20)]),
                   S([(w / 2 - 10, MID - 20 - H - G), (w + 20, -90)])])
    # S — разрыв посередине
    w = 690
    xl, xr = H, w - H
    glyph("S", w, [S([(xr + 10, TOP), (xl, TOP, R), (xl, MID, R), (w / 2 - G / 2, MID)]),
                   S([(w / 2 + G / 2, MID), (xr, MID, R), (xr, BOT, R), (-10, BOT)])])
    # T — верх отделён от ножки
    w = 680
    glyph("TТ", w, [hbar(0, w, TOP), stem(w / 2, -T, TOP - H - G)])
    # U
    w = 720
    xl, xr = H, w - H
    glyph("U", w, [S([(xl, CAP + T), (xl, BOT, RR), (xr, BOT, RR), (xr, CAP + T)])])
    # V
    w = 760
    glyph("V", w, [S([(H + 20, CAP + 80), (w / 2, -80), (w - H - 20, CAP + 80)])])
    # W
    w = 1000
    glyph("W", w, [S([(H + 10, CAP + 80), (w * 0.27, -80), (w / 2, CAP + 80), (w * 0.73, -80), (w - H - 10, CAP + 80)])])
    # X — одна диагональ с разрывом в центре
    w = 720
    k = G * 0.75
    glyph("XХ", w, [S([(H, CAP + 80), (w - H, -80)]),
                    S([(w - H, CAP + 80), (w / 2 + k, MID + k * CAP / w)]),
                    S([(w / 2 - k, MID - k * CAP / w), (H, -80)])])
    # Y — правая «ветка» не доходит до центра
    w = 720
    glyph("Y", w, [S([(H - 10, CAP + 80), (w / 2, MID - 10), (w / 2, -T)]),
                   S([(w - H + 10, CAP + 80), (w / 2 + G * 0.9, MID - 10 + G * 0.9)])])
    # Z
    w = 680
    glyph("Z", w, [S([(0, TOP), (w - H, TOP), (H, BOT), (w, BOT)])])

    # ---------- кириллица (то, что не совпало с латиницей) ----------
    # Б
    w = 680
    xl, xr = H, w - H
    glyph("Б", w, [S([(w, TOP), (xl, TOP), (xl, BOT), (xr, BOT, R), (xr, MID + 10, R), (xl, MID + 10)])])
    # Г
    w = 560
    xl = H
    glyph("Г", w, [S([(xl, -T), (xl, TOP), (w, TOP)])])
    # Д — с выносами вниз
    w = 760
    glyph("Д", w, [S([(150, BOT), (150, TOP), (w - H - 30, TOP), (w - H - 30, BOT)], band=(DESC, CAP)),
                   S([(0, BOT), (w, BOT)], band=(DESC, CAP)),
                   S([(H, BOT), (H, DESC - 10)], band=(DESC, CAP)),
                   S([(w - H, BOT), (w - H, DESC - 10)], band=(DESC, CAP))])
    # Ж — «уголки» отделены от средней ножки
    w = 1000
    c = w / 2
    glyph("Ж", w, [stem(c), S([(-30, CAP + 90), (c - H - G - 10, MID), (-30, -90)]),
                   S([(w + 30, CAP + 90), (c + H + G + 10, MID), (w + 30, -90)])])
    # З — средняя перекладина отделена слева
    w = 640
    xl, xr = H, w - H
    glyph("З", w, [S([(xl - 10, TOP), (xr - 25, TOP, R), (xr - 25, MID, R), (w * 0.34, MID)]),
                   S([(w * 0.34, MID), (xr, MID, R), (xr, BOT, R), (xl - 10, BOT)])])
    # И, Й
    w = 720
    xl, xr = H, w - H
    i_ = [S([(xl, CAP + T), (xl, BOT), (xr, TOP), (xr, -T)])]
    glyph("И", w, i_)
    glyph("Й", w, i_ + [S([(w / 2 - 130, 800), (w / 2 + 130, 800)], band=(-1000, 2000))])
    # Л
    w = 740
    xl, xr = H, w - H
    glyph("Л", w, [S([(H - 20, -90), (w * 0.34, TOP), (xr, TOP), (xr, -T)])])
    # П — верх отделён от правой ножки
    w = 700
    xl, xr = H, w - H
    glyph("П", w, [S([(xl, -T), (xl, TOP), (xr - H - G, TOP)]), stem(xr)])
    # У
    w = 700
    xl, xr = H, w - H
    glyph("У", w, [S([(xl, CAP + T), (xl, MID, R), (xr - H - G, MID)]),
                   S([(xr, CAP + T), (xr, BOT, R), (xl - 10, BOT)])])
    # Ф
    w = 880
    glyph("Ф", w, [stem(w / 2), S([(H, 150, R), (w - H, 150, R), (w - H, CAP - 150, R), (H, CAP - 150, R)], closed=True)])
    # Ц
    w = 760
    x2 = w - 120
    glyph("Ц", w, [S([(H, CAP + T), (H, BOT), (w, BOT)], band=(DESC, CAP)), stem(x2, BOT, CAP + T),
                   S([(w - H, BOT), (w - H, DESC - 10)], band=(DESC, CAP))])
    # Ч
    w = 680
    xl, xr = H, w - H
    glyph("Ч", w, [S([(xl, CAP + T), (xl, MID, R), (xr, MID)]), stem(xr)])
    # Ш — средняя ножка не доходит до низа
    w = 940
    xl, xr = H, w - H
    glyph("Ш", w, [S([(xl, CAP + T), (xl, BOT), (xr, BOT), (xr, CAP + T)]), stem(w / 2, BOT + H + G, CAP + T)])
    # Щ
    w = 1000
    x2 = w - 120
    glyph("Щ", w, [S([(H, CAP + T), (H, BOT), (w, BOT)], band=(DESC, CAP)), stem(x2, BOT, CAP + T),
                   stem((H + x2) / 2, BOT + H + G, CAP + T),
                   S([(w - H, BOT), (w - H, DESC - 10)], band=(DESC, CAP))])
    # Ь, Ъ, Ы
    def soft(x0, w):
        xl, xr = x0 + H, x0 + w - H
        return [stem(xl), S([(xl, BOT), (xr, BOT, R), (xr, MID + 10, R), (xl, MID + 10)])]
    glyph("Ь", 640, soft(0, 640))
    glyph("Ъ", 800, [hbar(0, 160 + T, TOP)] + soft(160, 640))
    glyph("Ы", 860, soft(0, 620) + [stem(860 - H)])
    # Э — средняя перекладина не доходит до правой стороны
    w = 700
    xl, xr = H, w - H
    glyph("Э", w, [S([(xl - 10, TOP), (xr, TOP, RR), (xr, BOT, RR), (xl - 10, BOT)]), hbar(w * 0.3, xr - H - G, MID)])
    # Ю
    w = 1000
    x0 = 250
    glyph("Ю", w, [stem(H), hbar(H, x0, MID),
                   S([(x0 + H, BOT, RR), (w - H, BOT, RR), (w - H, TOP, RR), (x0 + H, TOP, RR)], closed=True)])
    # Я — ножка отделена
    w = 690
    xl, xr = H, w - H
    glyph("Я", w, [stem(xr), S([(xr, TOP), (xl, TOP, R), (xl, MID - 20, R), (xr, MID - 20)]),
                   S([(w / 2 + 10, MID - 20 - H - G), (-20, -90)])])

    # ---------- цифры ----------
    w = 640
    xl, xr = H, w - H
    glyph("0", w, [S([(xl, BOT, RR), (xr, BOT, RR), (xr, TOP, RR), (xl, TOP, RR)], closed=True)])
    glyph("1", 380, [S([(20, TOP - 110), (380 - H, TOP + 40), (380 - H, -T)])])
    glyph("2", w, [S([(xl - 10, TOP), (xr, TOP, R), (xr, MID, R), (xl, MID, R), (xl, BOT), (w, BOT)])])
    # 3 — из двух выпуклостей (одной дугой она была бы неотличима от «Э»)
    glyph("3", w, [S([(xl - 10, TOP), (xr - 25, TOP, R), (xr - 25, MID, R), (w * 0.34, MID)]),
                   S([(w * 0.34, MID), (xr, MID, R), (xr, BOT, R), (xl - 10, BOT)])])
    glyph("4", 660, [S([(H, CAP + T), (H, MID - 40), (660, MID - 40)]), stem(660 - 150, -T, TOP - 60)])
    glyph("5", w, [S([(w, TOP), (xl, TOP), (xl, MID + 10), (xr, MID + 10, R), (xr, BOT, R), (xl - 10, BOT)])])
    glyph("6", w, [S([(w, TOP), (xl, TOP, R), (xl, BOT, R), (xr, BOT, R), (xr, MID + 10, R), (xl, MID + 10)])])
    glyph("7", 620, [S([(0, TOP), (620 - H, TOP), (260, -90)])])
    glyph("8", w, [S([(xl + 25, MID, R), (xr - 25, MID, R), (xr - 25, TOP, R), (xl + 25, TOP, R)], closed=True),
                   S([(xl, BOT, R), (xr, BOT, R), (xr, MID, R), (xl, MID, R)], closed=True)])
    glyph("9", w, [S([(0, BOT), (xr, BOT, R), (xr, TOP, R), (xl, TOP, R), (xl, MID - 10, R), (xr, MID - 10)])])

    # ---------- знаки ----------
    glyph(".", T, [stem(H, 0, T)])
    glyph(",", T, [S([(H, T), (H, -110)], band=(-110, CAP))])
    glyph(":", T, [stem(H, 110, 110 + T), stem(H, 470, 470 + T)])
    glyph("·", T, [stem(H, MID - H, MID + H)])
    glyph("-", 320, [hbar(0, 320, MID)])
    glyph("–", 520, [hbar(0, 520, MID)])
    glyph("—", 800, [hbar(0, 800, MID)])
    glyph("/", 440, [S([(0, -80), (440, CAP + 80)])])
    glyph("'", T, [stem(H, CAP - 200, CAP)])
    w = 300
    glyph("°", w, [S([(H * 0.7, CAP - w + H * 0.7, 60), (w - H * 0.7, CAP - w + H * 0.7, 60), (w - H * 0.7, CAP - H * 0.7, 60), (H * 0.7, CAP - H * 0.7, 60)], closed=True, t=T * 0.7, band=(-1000, 2000))])
    w = 760
    glyph("%", w, [S([(30, -80), (w - 30, CAP + 80)]),
                   S([(H * 0.7, CAP - 230, 50), (230, CAP - 230, 50), (230, CAP - H * 0.7, 50), (H * 0.7, CAP - H * 0.7, 50)], closed=True, t=T * 0.7, band=(-1000, 2000)),
                   S([(w - 230, H * 0.7, 50), (w - H * 0.7, H * 0.7, 50), (w - H * 0.7, 230, 50), (w - 230, 230, 50)], closed=True, t=T * 0.7, band=(-1000, 2000))])
    glyph("+", 520, [hbar(0, 520, MID), stem(260, MID - 260, MID + 260)])


def main():
    build_letters()
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    out = os.path.join(root, "assets", "fonts", "WallClockSplit.ttf")

    order = [".notdef", "space"]
    cmap = {32: "space", 160: "space"}
    glyphs, metrics = {}, {}

    # .notdef — пустой прямоугольник
    pen = TTGlyphPen(None)
    for (x0, y0, x1, y1) in [(SB, 0, SB + 500, CAP)]:
        pen.moveTo((x0, y0)); pen.lineTo((x0, y1)); pen.lineTo((x1, y1)); pen.lineTo((x1, y0)); pen.closePath()
        pen.moveTo((x0 + 60, y0 + 60)); pen.lineTo((x1 - 60, y0 + 60)); pen.lineTo((x1 - 60, y1 - 60)); pen.lineTo((x0 + 60, y1 - 60)); pen.closePath()
    glyphs[".notdef"] = pen.glyph()
    metrics[".notdef"] = (500 + 2 * SB, SB)
    glyphs["space"] = TTGlyphPen(None).glyph()
    metrics["space"] = (300, 0)

    done = {}
    for ch, (w, strokes) in G_.items():
        key = id(strokes)
        if key in done:
            name = done[key]
        else:
            name = "uni%04X" % ord(ch)
            pen = TTGlyphPen(None)
            xmin = 10 ** 9
            for s in strokes:
                for poly in stroke_contours(s):
                    pts = [(round(x + SB), round(y)) for x, y in poly]
                    clean = [p for i, p in enumerate(pts) if p != pts[i - 1]]
                    if len(clean) < 3:
                        continue
                    pen.moveTo(clean[0])
                    for p in clean[1:]:
                        pen.lineTo(p)
                    pen.closePath()
                    xmin = min(xmin, min(p[0] for p in clean))
            glyphs[name] = pen.glyph()
            metrics[name] = (round(w + 2 * SB), xmin if xmin < 10 ** 9 else 0)
            order.append(name)
            done[key] = name
        cmap[ord(ch)] = name
        # строчные буквы рисуем так же, как заглавные
        low = ch.lower()
        if low != ch and ord(low) not in cmap:
            cmap[ord(low)] = name

    fb = FontBuilder(UPM, isTTF=True)
    fb.setupGlyphOrder(order)
    fb.setupCharacterMap(cmap)
    fb.setupGlyf(glyphs)
    glyf = fb.font["glyf"]
    hmtx = {}
    for name in order:
        g = glyf[name]
        g.recalcBounds(glyf)
        lsb = getattr(g, "xMin", 0) if g.numberOfContours else 0
        hmtx[name] = (metrics[name][0], lsb)
    fb.setupHorizontalMetrics(hmtx)
    fb.setupHorizontalHeader(ascent=900, descent=-200)
    fb.setupNameTable({
        "familyName": "WallClock Split",
        "styleName": "Regular",
        "uniqueFontIdentifier": "WallClock Split Regular",
        "fullName": "WallClock Split",
        "psName": "WallClockSplit-Regular",
        "version": "Version 1.000",
        "copyright": "WallClock Split — шрифт для WallClock",
    })
    fb.setupOS2(
        sTypoAscender=800, sTypoDescender=-200, sTypoLineGap=0,
        usWinAscent=950, usWinDescent=250,
        sxHeight=CAP, sCapHeight=CAP,
        ulUnicodeRange1=(1 << 0) | (1 << 1) | (1 << 9),  # латиница, латиница-1, кириллица
        ulCodePageRange1=(1 << 0) | (1 << 2),             # кодовые страницы 1252 и 1251
        achVendID="WCLK",
    )
    fb.setupPost()
    fb.setupDummyDSIG = None
    fb.save(out)
    print("готово:", out, "глифов:", len(order))


if __name__ == "__main__":
    main()
