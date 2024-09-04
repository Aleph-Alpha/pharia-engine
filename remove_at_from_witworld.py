#!/usr/bin/python3
import sys

def doit(INPATH, OUTPATH):
    inlines = open(INPATH).readlines()
    print(f"read {INPATH}, {len(inlines)}")
    outlines = [line.strip("\n") for line in inlines if not line.strip().startswith("@")]
    open(OUTPATH, "w").write("\n".join(outlines))
    print(f"written {OUTPATH}, {len(outlines)}")
    print("You should copy  skill.wit  to sdk/wit/  but also adapt the python binding as well")

def main(args):
    VERSION = "@0.2"
    if args:
        VERSION = args[0]
    INPATH = f"wit/skill{VERSION}/skill.wit"
    OUTPATH = "skill.wit"
    doit(INPATH, OUTPATH)
    
if __name__ == '__main__':
    main(sys.argv[1:])
