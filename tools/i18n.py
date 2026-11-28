#!/usr/bin/env python3
"""Keeps the translation catalogs in step with the code.

  python3 tools/i18n.py          write po/whatsnap.pot and add every new string
                                 to each po/<code>.po with an empty translation
  python3 tools/i18n.py --check  only report what is missing or untranslated

Strings are collected from tr("..."), tr_str("..."), tr_fmt("...", ...) and the
`if cond { "A" } else { "B" }` form passed straight to them, plus the tables
listed in collect_code_strings(), such as `static ..._TEXTS` arrays. Other
strings that reach tr() through a variable are not found here, so entries
already in a .po file are never removed.
"""

import glob
import os
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
    backend = os.path.join("src", "i18n", "backend.rs")
    for path in glob.glob("src/**/*.rs", recursive=True):
        if path.startswith(os.path.join("src", "i18n", "")) and path != backend:
            continue
        content = open(path, encoding="utf-8").read()
        for m in call.finditer(content):
            for g in m.groups():
                if g:
                    found.add(unescape_rust(g))
        # Tables of English text named `static ..._TEXTS`, and the contact
        # picker's PickerWords, reach tr() through a variable.
        code = content.split("#[cfg(test)]")[0]
        blocks = re.findall(r"static\s+[A-Z_]+_TEXTS\s*:[^=]*=\s*\[(.*?)\];", code, re.DOTALL)
        blocks += re.findall(r"PickerWords\s*\{(.*?)\n\s*\}", code, re.DOTALL)
        for block in blocks:
            for m in re.finditer(LIT, block):
                found.add(unescape_rust(m.group(0)))

    palette = re.search(
        r"STATUS_PALETTE:\s*\[\(&str,\s*u32\);\s*\d+\]\s*=\s*\[(.*?)\];",
        open("src/ui/dialogs/status_composer.rs", encoding="utf-8").read(),
        re.DOTALL,
    )
    if palette:
        for m in re.finditer(r"\(\s*(" + LIT + r")\s*,", palette.group(1)):
            found.add(unescape_rust(m.group(1)))

    for s in re.findall(
        r"const\s+[A-Z0-9_]*SUFFIX\s*:\s*&str\s*=\s*(" + LIT + r");",
        open("src/ui_commands/calls.rs", encoding="utf-8").read(),
    ):
        found.add(unescape_rust(s))

    sd = open("src/ui/dialogs/settings_dialog.rs", encoding="utf-8").read()
    for pat in (
        r"notify_switch\(\s*(" + LIT + r")\s*,?\s*\)",
        r"(?:setting_switch|setting_note|section_header|setting_button)\([^;()]*,\s*(" + LIT + r")\s*,?\s*\)",
        r"visibility_combo\([^;()]*,\s*(" + LIT + r")\s*,",
        r"template_field\(\s*(" + LIT + r")\s*\)",
        r"sound_picker\(\s*(" + LIT + r")\s*,",
    ):
        for m in re.finditer(pat, sd):
            found.add(unescape_rust(m.group(1)))

    # src/i18n/backend.rs also replaces the parts in `for part in [...]`
    # lists, and passes by_who() a "You ..." and a "{} ..." form.
    be = open(backend, encoding="utf-8").read().split("#[cfg(test)]")[0]
    blocks = re.findall(r"for\s+part\s+in\s+\[(.*?)\]", be, re.DOTALL)
    blocks += re.findall(r"by_who\(\s*\w+,\s*(" + LIT + r"\s*,\s*" + LIT + ")", be)
    for block in blocks:
        for m in re.finditer(LIT, block):
            found.add(unescape_rust(m.group(0)))

    # The shortcut editor's tabs and action names, also used as menu labels.
    km = open("src/keymap.rs", encoding="utf-8").read()
    for m in re.finditer(r"\blabel:\s*(" + LIT + ")", km):
        found.add(unescape_rust(m.group(1)))
    tabs = re.search(r"pub const TABS:[^=]*=\s*\[(.*?)\];", km, re.DOTALL)
    if tabs:
        for m in re.finditer(LIT, tabs.group(1)):
            found.add(unescape_rust(m.group(0)))

    found.update([
        "Mobile", "iPhone", "Main", "Home fax", "Work fax", "Home", "Work",
        "Pager", "Email", "Description", "Business name",
    ])
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


if __name__ == "__main__":
    main()
