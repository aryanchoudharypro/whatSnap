#!/usr/bin/env python3
"""Keeps the translation catalogs in step with the code.

  python3 tools/i18n.py          write po/whatsnap.pot and add every new string
                                 to each po/<code>.po with an empty translation
  python3 tools/i18n.py --check  only report what is missing or untranslated

Strings are collected from tr("..."), tr_str("..."), tr_fmt("...", ...) and the
`if cond { "A" } else { "B" }` form passed straight to them. Strings that reach
tr() through a variable, such as table entries, are not found here, so entries
already in a .po file are never removed.
"""

import glob
import re
import sys

LIT = r'"(?:[^"\\]|\\.)*"'
ESC = {"n": "\n", "r": "\r", "t": "\t", "\\": "\\", '"': '"', "'": "'", "0": "\0"}


def unescape_rust(lit):
    body, out, i = lit[1:-1], "", 0
    while i < len(body):
        c = body[i]
        if c != "\\":
            out += c
            i += 1
            continue
        n = body[i + 1]
        if n == "\n":
            i += 2
            while i < len(body) and body[i] in " \t\r\n":
                i += 1
        elif n == "u":
            j = body.index("}", i)
            out += chr(int(body[i + 3 : j], 16))
            i = j + 1
        else:
            out += ESC.get(n, n)
            i += 2
    return out


def escape_po(text):
    return text.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n").replace("\r", "\\r").replace("\t", "\\t")


def unescape_po(text):
    out, i = "", 0
    while i < len(text):
        if text[i] == "\\" and i + 1 < len(text):
            out += {"n": "\n", "r": "\r", "t": "\t"}.get(text[i + 1], text[i + 1])
            i += 2
        else:
            out += text[i]
            i += 1
    return out


def collect_code_strings():
    found = set()
    call = re.compile(r"i18n::tr(?:_str|_fmt)?\(\s*(?:if [^{]+\{\s*(" + LIT + r")\s*\}\s*else\s*\{\s*)?(" + LIT + ")")
    for path in glob.glob("src/**/*.rs", recursive=True):
        if path.startswith("src/i18n/"):
            continue
        for m in call.finditer(open(path, encoding="utf-8").read()):
            for g in m.groups():
                if g:
                    found.add(unescape_rust(g))
    return found


def read_po(path):
    """msgid -> msgstr, plus the file text."""
    text = open(path, encoding="utf-8").read()
    entries, msgid, msgstr, field = {}, "", "", None
    for line in text.splitlines() + [""]:
        line = line.strip()
        if line.startswith("msgid "):
            if msgid:
                entries[msgid] = msgstr
            msgid, msgstr, field = unescape_po(line[7:-1]), "", "id"
        elif line.startswith("msgstr "):
            msgstr, field = unescape_po(line[8:-1]), "str"
        elif line.startswith('"') and field:
            part = unescape_po(line[1:-1])
            if field == "id":
                msgid += part
            else:
                msgstr += part
        elif not line:
            if msgid:
                entries[msgid] = msgstr
            msgid, msgstr, field = "", "", None
    return entries, text


def main():
    check = "--check" in sys.argv
    code = collect_code_strings()
    if not check:
        with open("po/whatsnap.pot", "w", encoding="utf-8") as f:
            f.write('msgid ""\nmsgstr ""\n"Content-Type: text/plain; charset=UTF-8\\n"\n\n')
            for s in sorted(code):
                f.write(f'msgid "{escape_po(s)}"\nmsgstr ""\n\n')
        print(f"po/whatsnap.pot: {len(code)} strings")
    bad = False
    for path in sorted(glob.glob("po/*.po")):
        entries, text = read_po(path)
        missing = sorted(s for s in code if s not in entries)
        untranslated = sum(1 for k, v in entries.items() if k and not v)
        print(f"{path}: {len(entries) - 1} entries, {len(missing)} new in code, {untranslated} untranslated")
        if missing and not check:
            with open(path, "a", encoding="utf-8") as f:
                f.write("\n" if not text.endswith("\n\n") else "")
                for s in missing:
                    f.write(f'msgid "{escape_po(s)}"\nmsgstr ""\n\n')
        bad = bad or bool(missing) or bool(untranslated)
    sys.exit(1 if check and bad else 0)


main()
