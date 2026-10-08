#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
"""Scan SDCC 8051 assembly for widened 8-bit differences.

SDCC 4.5 compiled `PAD_tick((uint8_t)(now - last))` (uint8 operands, uint16
parameter) as a 16-bit subtraction of the zero-extended operands and dropped
the cast: when `now < last` the high byte became 0xFF (firmware 2.0.1, raw mode
expired at every wrap of the 8-bit millisecond counter).

The signature in the generated code is a high-byte `subb` whose two operands
are both known to be zero and whose result is stored: a borrow from an 8-bit
subtraction carried into a wider value (a compare only uses the carry and is
fine). The same for a high-byte `addc` of zeros on a source line with an
explicit (u)int8_t cast: a carry from an 8-bit sum that the cast should drop. Every such site is reported with its C source line; a build fails
unless the line is in ALLOWED (reviewed: the wide, possibly negative result is
what the C code means).

    python asmcheck.py DIR          check every *.asm in DIR
    python asmcheck.py --self-test  the checker against known-good and known-bad code
"""
import os
import re
import sys

# "file.c:line" -> why the widened difference is intended there.
ALLOWED: dict[str, str] = {}

SRC = re.compile(r"^;\s+(\S+\.c):(\d+):\s?(.*)$")
MOV_ZERO = re.compile(r"^\s+mov\s+(r[0-7]|a),#0x00\s*$")
CLR_A = re.compile(r"^\s+clr\s+a\s*$")
MOV_A_REG = re.compile(r"^\s+mov\s+a,(r[0-7])\s*$")
MOV_REG_A = re.compile(r"^\s+mov\s+(r[0-7]),a\s*$")
SUBB = re.compile(r"^\s+subb\s+a,(r[0-7]|#0x00)\s*$")
ADDC = re.compile(r"^\s+addc\s+a,\s*(r[0-7]|#0x00)\s*$")
NARROW_CAST = re.compile(r"\((u?int8_t)\)")
STORE_A = re.compile(r"^\s+(mov\s+[^,]+,a|movx\s+@\w+,a|push\s+acc)\s*$")
BARRIER = re.compile(r"^(\w+\$?:|\s+(lcall|ljmp|sjmp|ret|reti|jz|jnz|jc|jnc|cjne|djnz)\b)")


def scan(path: str):
    hits = []
    zero: set[str] = set()      # registers known to hold 0 in this basic block
    src = ("?", 0, "")
    pending = None              # a borrow-only high byte, waiting for its use
    for line in open(path, encoding="utf-8", errors="replace"):
        m = SRC.match(line)
        if m:
            src = (os.path.basename(m.group(1)), int(m.group(2)), m.group(3).strip())
            continue
        if not line.strip() or line.lstrip().startswith(";"):
            continue
        if pending is not None:
            # Only a stored result is a value; a compare uses the borrow (carry).
            if STORE_A.match(line):
                hits.append(pending)
            pending = None
        if BARRIER.match(line):
            zero.clear()
            continue
        if MOV_ZERO.match(line):
            zero.add(MOV_ZERO.match(line).group(1))
            continue
        if CLR_A.match(line):
            zero.add("a")
            continue
        m = MOV_A_REG.match(line)
        if m:
            if m.group(1) in zero:
                zero.add("a")
            else:
                zero.discard("a")
            continue
        m = MOV_REG_A.match(line)
        if m:
            if "a" in zero:
                zero.add(m.group(1))
            else:
                zero.discard(m.group(1))
            continue
        m = ADDC.match(line)
        if m:
            other = m.group(1)
            # A carry from an 8-bit sum widened past an explicit 8-bit cast.
            if "a" in zero and (other == "#0x00" or other in zero) and NARROW_CAST.search(src[2]):
                pending = src
            zero.discard("a")
            continue
        m = SUBB.match(line)
        if m:
            other = m.group(1)
            if "a" in zero and (other == "#0x00" or other in zero):
                pending = src
            zero.discard("a")
            continue
        # Anything else writing a register forgets what we knew about it.
        w = re.match(r"^\s+\w+\s+(r[0-7]|a)\b", line)
        if w:
            zero.discard(w.group(1))
    return hits


# Fixtures for --self-test: real SDCC 4.5 output.
FIXTURES = {
    # 2.0.1's main loop: PAD_tick((uint8_t)(now - last)) with a uint16 parameter.
    "bad_201.asm": ("""
;	padfw.c:199: PAD_tick((uint8_t)(now - last));
	mov	ar4,r6
	mov	r5,#0x00
	mov	ar2,r7
	mov	r3,#0x00
	mov	a,r4
	clr	c
	subb	a,r2
	mov	r4,a
	mov	a,r5
	subb	a,r3
	mov	r5,a
	mov	dpl,r4
	mov	dph,r5
	lcall	_PAD_tick
""", 1),
    # A compare: the borrow only feeds the carry.
    "ok_compare.asm": ("""
;	padlogic.c:113: return (code & 0xFF) < LAYER_COUNT;
	mov	r6,#0x00
	clr	c
	mov	a,r7
	subb	a,#0x02
	mov	a,r6
	subb	a,#0x00
	mov	_x,c
""", 0),
    # 2.0.2: the difference stays 8 bits.
    "ok_202.asm": ("""
;	padfw.c:211: elapsed -= last;
	mov	a,r6
	clr	c
	subb	a,r7
	mov	dpl,a
	lcall	_PAD_tick
""", 0),
    # A carry from an 8-bit sum stored past an explicit cast.
    "bad_carry.asm": ("""
;	x.c:5: f16((uint8_t)(a + b));
	mov	r5,#0x00
	mov	r3,#0x00
	mov	a,r4
	add	a,r2
	mov	r4,a
	mov	a,r5
	addc	a,r3
	mov	r5,a
""", 1),
}


def self_test() -> int:
    import tempfile
    failed = 0
    with tempfile.TemporaryDirectory() as d:
        for name, (text, want) in FIXTURES.items():
            path = os.path.join(d, name)
            with open(path, "w") as fh:
                fh.write(text)
            got = len(scan(path))
            ok = got == want
            failed += not ok
            print(f"asmcheck self-test: {name}: {got} site(s), want {want}: {'ok' if ok else 'FAIL'}")
    return 1 if failed else 0


def main() -> int:
    if sys.argv[1:] == ["--self-test"]:
        return self_test()
    d = sys.argv[1] if len(sys.argv) > 1 else "."
    bad = 0
    for name in sorted(os.listdir(d)):
        if not name.endswith(".asm"):
            continue
        for f, n, text in scan(os.path.join(d, name)):
            key = f"{f}:{n}"
            if key in ALLOWED:
                print(f"asmcheck: ok    {key}: {text}  ({ALLOWED[key]})")
            else:
                print(f"asmcheck: WIDENED 8-BIT DIFFERENCE {key}: {text}")
                bad += 1
    if bad:
        print(f"asmcheck: {bad} site(s): an 8-bit difference reaches a wider value through a borrow; "
              "compute it into a uint8_t variable (or pass a uint8_t), or review and allow it", file=sys.stderr)
        return 1
    print("asmcheck: no widened 8-bit differences")
    return 0


if __name__ == "__main__":
    sys.exit(main())
