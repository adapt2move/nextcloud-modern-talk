#!/usr/bin/env python3
"""Package a source-bound Linux diagnostic helper; never a finished app release.

Runs in the assigned build VM. Uses upstream's GN dependency license mapping,
fails on missing notices, and creates deterministic archives of existing inputs.
Native compilation itself is not yet independently bit-for-bit reproduced.
"""
import argparse
import gzip
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import tarfile
import tempfile

REPO=Path(__file__).resolve().parents[2]


def sha(path):
    h=hashlib.sha256()
    with Path(path).open("rb") as f:
        for block in iter(lambda:f.read(1024*1024),b""):h.update(block)
    return h.hexdigest()


def archive(path,files):
    with path.open("wb") as raw,gzip.GzipFile(fileobj=raw,mode="wb",filename="",mtime=0) as compressed,tarfile.open(fileobj=compressed,mode="w") as tar:
        for name,source,executable in sorted(files):
            data=Path(source).read_bytes()
            info=tarfile.TarInfo(name);info.size=len(data);info.mode=0o755 if executable else 0o644
            info.uid=info.gid=info.mtime=0;info.uname=info.gname=""
            tar.addfile(info,io.BytesIO(data))


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument("--root",type=Path,required=True);p.add_argument("--driver",type=Path,required=True);p.add_argument("--output",type=Path,required=True)
    p.add_argument("--build-evidence",type=Path,required=True)
    p.add_argument("--verification",type=Path,required=True)
    p.add_argument("--native-output-name",required=True)
    args=p.parse_args();root=args.root.resolve();out=args.output.resolve();src=root/"src"
    if platform.system()!="Linux" or platform.machine()!="x86_64":raise SystemExit("Only Linux x86_64 diagnostic packaging has an implemented profile")
    if out.is_relative_to(REPO) or REPO.is_relative_to(out):raise SystemExit("External artifact directory required")
    if out.exists():raise SystemExit("New output directory required; do not overwrite evidence")
    out.mkdir(parents=True)
    if not args.native_output_name.replace('-','').replace('_','').isalnum():raise SystemExit("Bounded output name required")
    evidence=args.build_evidence.resolve();native_output=src/"out"/args.native_output_name
    build=json.loads((evidence/"build.json").read_text());binary=native_output/"nmt-media-helper"
    checks=json.loads(args.verification.read_text())
    if checks.get("result")!="pass" or checks.get("helper_sha256")!=sha(binary) or checks.get("driver_sha256")!=sha(args.driver):raise SystemExit("Source-bound verification/artifact mismatch")
    if sha(binary)!=build["binary_sha256"]:raise SystemExit("Helper differs from build identity")
    if sha(native_output/"args.gn")!=build["gn_args_sha256"]:raise SystemExit("GN output differs from build identity")
    for path,digest in build["inputs"].items():
        if sha(REPO/path)!=digest:raise SystemExit("Rebuild after source change: "+path)
    license_script=src/"tools_webrtc/libs/generate_licenses.py"
    os.environ["PATH"]=str(root/"depot_tools")+os.pathsep+os.environ["PATH"]
    os.environ["DEPOT_TOOLS_UPDATE"]="0"
    sys.path.insert(0,str(license_script.parent))
    spec=importlib.util.spec_from_file_location("upstream_license",license_script)
    module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
    builder=module.LicenseBuilder([str(native_output)],["//nmt_media:nmt-media-helper"])
    libraries=builder._get_third_party_libraries(str(native_output),"//nmt_media:nmt-media-helper")|{"webrtc"}
    missing=libraries-set(builder.common_licenses_dict)
    if missing:raise SystemExit("Unmapped GN license dependency")
    with tempfile.TemporaryDirectory(prefix="nmt-media-package-",dir=out) as temporary:
        temporary=Path(temporary);builder.generate_license_text(str(temporary))
        notices=[];files=[("bin/nmt-media-helper",binary,True),("bin/nmt-media-driver",args.driver,True),
                        ("LICENSE",REPO/"LICENSE",False),("THIRD-PARTY-NOTICES.md",temporary/"LICENSE.md",False)]
        for library in sorted(libraries):
            paths=builder.common_licenses_dict[library]
            notices.append({"dependency":library,"notice_files":[{"path":p,"sha256":sha(src/p)} for p in paths],"upstream_compile_time_only":not paths})
            for path in paths:files.append(("notices/"+path,src/path,False))
        for path in ["PATENTS","AUTHORS","third_party/libvpx/source/libvpx/PATENTS"]:
            files.append(("notices/"+path,src/path,False))
        for source in [root/"fetch-identity.json",root/"dependency-revisions.json",evidence/"build-input.json",evidence/"build.json",evidence/"patches.json",evidence/"upstream.patch"]:
            files.append(("provenance/"+source.name,source,False))
        # GN is an over-approximation of linked reachability, not an SDP-codec
        # allowlist. Other codecs/SFrame/etc compiled by the aggregate are named.
        inventory={"schema":1,"profile":"Linux x86_64 diagnostic-synthetic only","distribution_approval":False,
            "license_method":"upstream pinned GN dependency mapping; conservative build closure, not symbol reachability or complete legal clearance",
            "license_generator_sha256":sha(license_script),"dependencies":notices,
            "webrtc_dependencies_sha256":sha(root/"dependency-revisions.json"),
            "helper_sha256":sha(binary),"driver_sha256":sha(args.driver),
            "ipc_version":2,"sctp":"bounded private reliable ordered status/simplewebrtc; not general application messaging",
            "verification_sha256":sha(args.verification),"native_build_sha256":sha(evidence/"build.json"),
            "project_rust":"1.99.1","project_lock_sha256":sha(REPO/"Cargo.lock"),
            "system_runtime":"ELF dynamically uses glibc/libm/libgcc_s/libdl/libpthread; sysroot is DEPS-pinned, distribution supplies compatible runtime",
            "build_reproduction":"exact pinned recipe; independent clean binary reproduction pending"}
        (temporary/"inventory.json").write_text(json.dumps(inventory,indent=2)+"\n")
        files.append(("inventory.json",temporary/"inventory.json",False))
        (temporary/"README.txt").write_text("NMT native media diagnostic, not an installable Nextcloud client.\nExtract and run bin/nmt-media-helper --version. The helper otherwise requires the\nRust supervisor and an explicit synthetic diagnostic lifetime; no native device\nor production Talk authority exists. Read native/media/README.md in source.\nNo macOS artifact, signing, notarization, app UI or release approval is included.\n")
        files.append(("README.txt",temporary/"README.txt",False))
        artifact=out/"nmt-media-linux-x86_64-diagnostic.tar.gz";archive(artifact,files)
        shutil.copyfile(temporary/"inventory.json",out/"inventory.json")
        sources=[]
        for base in ["native/media","tools/media","crates/talk-adapters/src/media"]:
            for path in sorted((REPO/base).rglob("*")):
                if path.is_file() and "__pycache__" not in path.parts:sources.append((str(path.relative_to(REPO)),path,False))
        for path in ["crates/talk-adapters/src/media.rs","LICENSE","rust-toolchain.toml"]:sources.append((path,REPO/path,False))
        source_archive=out/"nmt-media-project-source.tar.gz";archive(source_archive,sources)
        manifest={"schema":1,"artifacts":[{"file":path.name,"bytes":path.stat().st_size,"sha256":sha(path)} for path in (artifact,source_archive)],"input_files":{name:sha(source) for name,source,_ in files},"source_files":{name:sha(source) for name,source,_ in sources}}
        (out/"artifacts.json").write_text(json.dumps(manifest,indent=2)+"\n")
    print(json.dumps({"result":"packaged diagnostic only","artifacts":manifest["artifacts"]}))


if __name__=="__main__":main()
