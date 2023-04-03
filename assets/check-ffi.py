import sys

import re

with open('crate/multiworld-csharp/src/lib.rs', encoding='utf-8') as rs_f:
    rs = rs_f.read()
with open('crate/multiworld-bizhawk/OotrMultiworld/src/MainForm.cs', encoding='utf-8') as cs_f:
    cs = cs_f.read()
rs_fns = set()
cs_fns = set()
for line in rs.splitlines():
    if match := re.search('#\\[csharp_ffi\\] pub (?:unsafe )?extern "C" fn ([0-9a-z_]+)\\(', line):
        rs_fns.add(match.group(1))
for line in cs.splitlines():
    if match := re.search('        \\[DllImport\\("multiworld"\\)\\] internal static extern [0-9A-Za-z_]+ ([0-9a-z_]+)\\(', line):
        cs_fns.add(match.group(1))
okay = True
for cs_fn in cs_fns - rs_fns:
    print(f'only in C#: {cs_fn}')
    okay = False
if not okay:
    sys.exit(1)
