"""Rewrite tuple records `record((...)).build(|..| T { .. })` into keyed records."""
import re, sys

def match_paren(s, i):
    """s[i] is an opening bracket; return the index of its match."""
    depth=0; j=i; in_str=False
    while j < len(s):
        c=s[j]
        if in_str:
            if c=='\\': j+=2; continue
            if c=='"': in_str=False
        elif c=='"': in_str=True
        elif c in '([{': depth+=1
        elif c in ')]}':
            depth-=1
            if depth==0: return j
        j+=1
    raise ValueError("unbalanced")

def split_top(s):
    """split on top-level commas"""
    parts=[]; depth=0; cur=''; in_str=False
    for c in s:
        if in_str:
            cur+=c
            if c=='"': in_str=False
            continue
        if c=='"': in_str=True; cur+=c; continue
        if c in '([{': depth+=1
        if c in ')]}': depth-=1
        if c==',' and depth==0:
            parts.append(cur.strip()); cur=''
        else: cur+=c
    if cur.strip(): parts.append(cur.strip())
    return parts

def collect_fields(inner):
    out=[]
    for p in split_top(inner):
        if p.startswith('('):  # nested group
            out+=collect_fields(p[1:match_paren(p,0)])
            continue
        m=re.match(r'(field|optional_field|field_default)\(', p)
        assert m, p
        kind=m.group(1)
        args=split_top(p[m.end():match_paren(p, m.end()-1)])
        name=args[0]; codec=args[1]; getter=args[2]
        gm=re.search(r'&\s*([a-z])\.(\w+)', getter)
        fld=gm.group(2)
        extra=args[3:]
        out.append((kind,name,codec,fld,extra))
    return out

def transform(src):
    res=''; pos=0
    while True:
        i=src.find('record((', pos)
        if i<0: res+=src[pos:]; break
        res+=src[pos:i]
        open_i=i+len('record')
        close_i=match_paren(src, open_i)
        inner=src[open_i+2:close_i-1] if src[close_i-1]==')' else None
        tup=src[open_i+1:close_i]   # "( ... )"
        tup_inner=tup[1:match_paren(tup,0)]
        fields=collect_fields(tup_inner)
        b=src.index('.build(', close_i)
        bclose=match_paren(src, b+len('.build'))
        build=src[b+len('.build('):bclose]
        tm=re.search(r"\|\s*\{?\s*([A-Z]\w*)\s*\{", build)
        if not tm: print("FAILED build:", repr(build[:300])); raise SystemExit(1)
        T=tm.group(1)
        # trailing modifiers after build e.g. .allow_unknown()
        indent=re.search(r'\n([ \t]*)$', res).group(1) if re.search(r'\n([ \t]*)$', res) else '    '
        lines=[f'let r = keyed_record::<{T}>();']
        for kind,name,codec,fld,extra in fields:
            meth={'field':'field','optional_field':'optional_field','field_default':'field_default'}[kind]
            ex=''.join(', '+e for e in extra)
            lines.append(f'let (r, k_{fld}) = r.{meth}({name}, {codec}, |x| &x.{fld}{ex});')
        lit=', '.join(f'{fld}: v.take(k_{fld})' for _,_,_,fld,_ in fields)
        lines.append(f'r.build(move |v| {T} {{ {lit} }})')
        res+=('\n'+indent).join(lines)
        pos=bclose+1
    return res

src=open(sys.argv[1]).read()
open(sys.argv[2],'w').write(transform(src))
