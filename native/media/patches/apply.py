#!/usr/bin/env python3
"""Deterministic patches to the exact ADR-005 checkout; refuses changed inputs.

The source revision check is performed by build.py. Original contents are read
from the matching Git object, never adopted from a modified working file.
"""
import difflib
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import subprocess


def apply(src: Path, output: Path):
    changes = []
    diffs = []
    previous = {}
    if (output / "patches.json").exists():
        previous = {entry["path"]: entry["after_sha256"] for entry in json.loads((output / "patches.json").read_text())["files"]}

    def modify(repo, relative, transform):
        path = repo / relative
        old = subprocess.check_output(["git", "-C", str(repo), "show", "HEAD:" + relative])
        new = transform(old.decode()).encode()
        if new == old:
            raise RuntimeError("Patch did not change " + relative)
        label = str(path.relative_to(src))
        current = path.read_bytes()
        if current not in (old, new) and hashlib.sha256(current).hexdigest() != previous.get(label):
            raise RuntimeError("Unexpected local edit at " + relative)
        if current != new:
            path.write_bytes(new)
        diffs.extend(difflib.unified_diff(old.decode().splitlines(True), new.decode().splitlines(True),
                                       fromfile="a/" + label, tofile="b/" + label))
        changes.append({"path": label, "before_sha256": hashlib.sha256(old).hexdigest(),
                        "after_sha256": hashlib.sha256(new).hexdigest()})

    def sockets(text):
        declaration = '\nextern "C" bool nmt_media_network_allowed();\nextern "C" void nmt_media_network_sent();\n'
        needle = "int PhysicalSocket::Send(const void* pv, size_t cb) {"
        if text.count(needle) != 1:
            raise RuntimeError("Socket source changed")
        text = text.replace(needle, declaration + needle + '\n  if (!nmt_media_network_allowed()) { SetError(EACCES); return -1; }')
        needle2 = "const SocketAddress& addr) {\n  sockaddr_storage saddr;"
        if text.count(needle2) != 1:
            raise RuntimeError("Datagram source changed")
        text = text.replace(needle2, "const SocketAddress& addr) {\n  if (!nmt_media_network_allowed()) { SetError(EACCES); return -1; }\n  sockaddr_storage saddr;")
        # Both successful socket writes, including retransmitted/TURN traffic.
        segment_start = text.index(declaration)
        segment_end = text.index("int PhysicalSocket::Recv(", segment_start)
        segment = text[segment_start:segment_end]
        if segment.count("  return sent;") != 2:
            raise RuntimeError("Send result source changed")
        segment = segment.replace("  return sent;", "  if (sent > 0) nmt_media_network_sent();\n  return sent;")
        return text[:segment_start] + segment + text[segment_end:]

    modify(src, "rtc_base/physical_socket_server.cc", sockets)
    def no_sctp(text):
        # This pin otherwise attempts to create a disabled data transport while
        # applying a remote offer, before it can reject that optional m-section.
        # Preserve the remote SDP; answer port zero, with no SCTP factory/backing.
        begin = text.index("void SdpOfferAnswerHandler::GetOptionsForUnifiedPlanAnswer(")
        end = text.index("const char* SdpOfferAnswerHandler::SessionErrorToString", begin)
        part = text[begin:end]
        needle = "      RTC_CHECK_EQ(MediaType::DATA, media_type);"
        if part.count(needle) != 1:
            raise RuntimeError("Unified Plan answer source changed")
        part = part.replace(needle, needle + "\n#ifndef WEBRTC_HAVE_SCTP\n      session_options->media_description_options.push_back(\n          GetMediaDescriptionOptionsForRejectedData(content.mid()));\n      continue;\n#endif")
        text = text[:begin] + part + text[end:]
        needle = "RTCError SdpOfferAnswerHandler::UpdateDataChannelTransport(\n    ContentSource source,\n    const ContentInfo& content,\n    const ContentGroup* bundle_group) {"
        if text.count(needle) != 1:
            raise RuntimeError("Data transport source changed")
        return text.replace(needle, needle + "\n#ifndef WEBRTC_HAVE_SCTP\n  // No transport can be created; the answer explicitly rejects this section.\n  return RTCError::OK();\n#endif")
    modify(src, "pc/sdp_offer_answer.cc", no_sctp)
    spec = importlib.util.spec_from_file_location("nmt_sctp_patch", Path(__file__).with_name("sctp.py"))
    sctp = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(sctp)
    sctp.apply(src, modify)
    modify(src, "BUILD.gn", lambda s: s.replace('    deps = [ ":webrtc" ]', '    deps = [ ":webrtc", "//nmt_media:nmt-media-helper" ]', 1)
           .replace('      "//:webrtc_lib_link_test",', '      "//:webrtc_lib_link_test",\n      "//nmt_media:nmt-media-helper",', 1))
    third = src / "third_party"
    config = third / "libvpx/source/config"
    count = 0
    for path in sorted(config.rglob("vpx_config.*")):
        if path.suffix not in (".h", ".asm"):
            continue
        text = path.read_text()
        if "DECODE_WIDTH_LIMIT" not in text:
            continue
        def limits(text):
            text, w = re.subn(r"((?:define|equ) DECODE_WIDTH_LIMIT[ ,]+)\d+", r"\g<1>1920", text)
            text, h = re.subn(r"((?:define|equ) DECODE_HEIGHT_LIMIT[ ,]+)\d+", r"\g<1>1080", text)
            if w != 1 or h != 1 or not re.search(r"(?:define|equ) CONFIG_SIZE_LIMIT[ ,]+1", text):
                raise RuntimeError("Unexpected libvpx size-limit configuration")
            return text
        modify(third, str(path.relative_to(third)), limits)
        count += 1
    if count < 2:
        raise RuntimeError("Missing libvpx configurations")
    modify(third, "libvpx/generate_gni.sh", lambda s: s.replace("--size-limit=16384x16384", "--size-limit=1920x1080"))
    output.mkdir(parents=True, exist_ok=True)
    (output / "upstream.patch").write_text("".join(diffs))
    (output / "patches.json").write_text(json.dumps({"files": changes,
        "patch_sha256": hashlib.sha256((output / "upstream.patch").read_bytes()).hexdigest()}, indent=2) + "\n")
