#!/usr/bin/env python3
"""Analyze immutable recorded getStats samples with the corrected codec predicate.

This does not execute another call or rewrite original reports. Missing/null
codec entries contribute no evidence. Native/media counters and all thresholds
remain unchanged; an original failed execution stays labeled as failed.
"""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import re

from talk_browser import media_checks
import talk_browser


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def read(path, limit):
    raw=path.read_bytes()
    if len(raw)>limit:
        raise ValueError("analysis input bound")
    return json.loads(raw)


def negatives(samples):
    expected=media_checks(samples)
    assert all(expected.values()), "positive actual recorded sample required"
    null=copy.deepcopy(samples)
    null[0].append({"rows":[{"type":"inbound-rtp","kind":"video","codec":None,"framesDecoded":999}],"decoded_audio_peak_rms":0})
    assert media_checks(null)==expected
    for mode in ("missing","null","wrong-type","unsupported-codec"):
        sample=copy.deepcopy(samples)
        for peers in sample:
            for peer in peers:
                for row in peer["rows"]:
                    if mode=="missing":row.pop("codec",None)
                    else:row["codec"]=None if mode=="null" else "audio/PCMU" if mode=="unsupported-codec" else 1
        assert not any(media_checks(sample).values())
    silent=copy.deepcopy(samples)
    for peers in silent:
        for peer in peers:peer["decoded_audio_peak_rms"]=0
    assert not media_checks(silent)["received_native_opus_audio"]
    below=copy.deepcopy(samples)
    for peers in below:
        for peer in peers:
            for row in peer["rows"]:
                row.update(framesDecoded=19,framesEncoded=19,packetsReceived=24,packetsSent=24)
    assert not any(media_checks(below).values())
    return ["unresolved-row-does-not-erase-valid-evidence","all-missing-no-positive","all-null-no-positive",
            "wrong-type-no-positive","unsupported-codec-no-positive","packets-without-decoded-remote-RMS-fail",
            "below-threshold-resolved-codec-fails"]


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--matrix",type=Path,required=True)
    parser.add_argument("--output",type=Path,required=True)
    args=parser.parse_args()
    if args.output.exists():raise SystemExit("new output required")
    matrix=read(args.matrix,65536)
    if len(matrix["runs"])>14 or matrix["result"]=="running":raise SystemExit("completed bounded matrix required")
    report={"schema":1,"classification":"offline observer analysis; original executions unchanged",
            "analyzer_sha256":sha(__file__),"observer_sha256":sha(talk_browser.__file__),
            "matrix_sha256":sha(args.matrix),"original_matrix_result":matrix["result"],
            "new_call_executions":0,"runs":[]}
    positive=None
    for entry in matrix["runs"]:
        name=entry["file"]
        if not re.fullmatch(r'r(?:34|35)-(?:exchange-[1-6]|remote-leave-1)\.json',name):raise ValueError("input name")
        path=args.matrix.parent/name
        if sha(path)!=entry["sha256"]:raise ValueError("original report changed")
        original=read(path,2*1024*1024)
        samples=original.get("browser_samples",[])
        if len(samples)>64 or any(len(peers)>8 for peers in samples):raise ValueError("sample count")
        checks=media_checks(samples)
        native=original.get("native",{})
        row={"file":name,"original_sha256":sha(path),"original_result":original["result"],
             "original_failure_type":original.get("failure_type"),"family":entry["family"],
             "scenario":entry["scenario"],"native_offerer":native.get("native_offerer"),
             "browser_checks":checks,"native_result":native.get("result"),
             "media_criteria_observed":all(checks.values()) and native.get("result")=="pass",
             "recorded_samples_canonical_json_sha256":hashlib.sha256(json.dumps(samples,sort_keys=True,separators=(",",":")).encode()).hexdigest(),
             "sample_hash_format":"UTF-8 json.dumps(sort_keys=True,separators=(',',':')); all recorded values unchanged",
             "observation":"complete for stated browser thresholds" if all(checks.values()) else "incomplete for stated browser thresholds",
             "missing_or_null_codec_rows":sum(row.get("codec") is None for peers in samples for peer in peers for row in peer["rows"]),
             "room_cleanup":original.get("room_cleanup"),"departure":native.get("remote_departure"),
             "stop":native.get("stop"),"leave":native.get("leave"),"deactivate":native.get("deactivate")}
        if all(checks.values()) and positive is None:positive=samples
        if original.get("browser_checks") is not None:
            assert original["browser_checks"]==checks, "correction changed a resolved-codec verdict"
        row["corrected_observer_exception_only"]=(original["result"]=="fail" and original.get("failure_type")=="AttributeError"
            and original.get("stage")=="native browser exchange" and row["media_criteria_observed"]
            and original.get("room_cleanup")=="absence_confirmed")
        report["runs"].append(row)
    if positive is None:raise ValueError("no actual positive for regression")
    report["negative_regressions"]=negatives(positive)
    report["actual_role_cells"]=[{"family":family,"native_offerer":role,
        "observed_media_count":sum(r["family"]==family and r["scenario"]=="exchange" and r["native_offerer"] is role and r["media_criteria_observed"] for r in report["runs"])}
        for family in ("r34","r35") for role in (True,False)]
    report["analysis_complete"]=True
    args.output.write_text(json.dumps(report,indent=2)+"\n")
    print(json.dumps({"analysis_complete":True,"runs":len(report["runs"]),"new_call_executions":0}))


if __name__=="__main__":main()
