#!/usr/bin/env python3
"""Bounded synthetic-room ownership/lost-response regression, on the media VM.

No call, media, poll, timeout change or POST replay. The optional old harness
comparison injects loss AFTER a real successful create, then independently
reconciles only that test's unique room. Original latency is not reproduced by
this fault injection. Never exports names, tokens, credentials or raw bodies.
"""
import argparse
import asyncio
import hashlib
import importlib.util
import json
from pathlib import Path
import secrets
import tempfile
from urllib.parse import urlparse

import talk_browser as current


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def load(path):
    spec = importlib.util.spec_from_file_location("old_talk_browser", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


async def negatives():
    records = []
    with tempfile.TemporaryDirectory(prefix="nmt-room-checks-", dir="/run") as scratch:
        for label, rows, expected in (
            ("ambiguous-owned", "duplicate", "reject"),
            ("wrong-owner", "wrong-owner", "reject"),
            ("oversized-list", "oversize", "reject"),
            ("unknown-create-absent", "absent", "uncertain_create"),
            ("delete-uncertain-no-replay", "present", "uncertain_delete"),
        ):
            room = current.FreshRoom("http://127.0.0.1:8794", "synthetic-unused", Path(scratch)/label)
            room.state = "create_sent_once"
            room.persist()
            commands = []
            def request(method, suffix, values=None):
                commands.append(method)
                row = {"token":"syntheticToken","name":room.name,"type":2,"participantType":1}
                if method == "DELETE":
                    raise TimeoutError("injected lost response")
                if rows == "duplicate":
                    return [row, dict(row,token="secondSyntheticToken")]
                if rows == "wrong-owner":
                    return [dict(row,participantType=3)]
                if rows == "oversize":
                    return [dict(row,token=f"synthetic{i}") for i in range(33)]
                if rows == "absent":
                    return [{"token":"unrelatedToken","name":"unrelated","type":2,"participantType":1}]
                return [row]
            room.request = request
            try:
                actual = await room.close()
            except RuntimeError:
                actual = "reject"
            if rows == "present":
                assert await room.close() == "uncertain_delete"
                assert commands.count("DELETE") == 1
            else:
                assert "DELETE" not in commands
            assert actual == expected and room.journal.exists()
            records.append({"case":label,"result":"pass","delete_requests":commands.count("DELETE"),
                            "unresolved_journal_retained":True})
    return records


async def actual(manifest_path, old):
    manifest = current.private_json(manifest_path)
    credentials = current.private_json(manifest["credentials_file"])
    origin, target = urlparse(manifest["client_url"]), urlparse(manifest["vm_url"])
    if origin.hostname != "127.0.0.1" or origin.scheme != "http" or target.scheme != "http":
        raise RuntimeError("owned loopback lab required")
    result = {"family":manifest["profile"],"request_timeout_seconds":3,"create_replay":False}
    async with current.LoopbackForward(target.hostname, origin.port):
        name = "nmt-lost-response-before-" + secrets.token_hex(12)
        journal = manifest_path.parent / "media-probe-owner.json"
        journal.write_text(json.dumps({"name":name,"state":"before-create-sent-once"}))
        journal.chmod(0o600)
        before = old.FreshRoom(manifest["client_url"],credentials["users"]["lab_alice"])
        old_request = before.request
        posts = 0
        def lose_old(method,suffix,values=None):
            nonlocal posts
            if method == "POST" and suffix == "":
                posts += 1
                data = old_request(method,suffix,dict(values,roomName=name))
                assert isinstance(data,dict) and data.get("name") == name
                raise TimeoutError("injected after successful server create")
            return old_request(method,suffix,values)
        before.request = lose_old
        try:
            await before.open()
            raise AssertionError("loss not injected")
        except TimeoutError:
            pass
        await before.close()
        rows = await asyncio.to_thread(old_request,"GET","?noStatusUpdate=1&includeLastMessage=false")
        assert isinstance(rows,list) and len(rows)<=32
        matches = [row for row in rows if row.get("name")==name and row.get("type")==2 and row.get("participantType")==1]
        assert posts==1 and before.token is None and len(matches)==1
        token=matches[0]["token"]
        assert current.FreshRoom.valid_token(token)
        result["before"]={"lost_create_response":True,"create_requests":posts,"old_close_sent_delete":False,
                          "owned_room_still_present":True,"false_cleanup_claim_reproduced":True}
        await asyncio.to_thread(old_request,"DELETE","/"+token)
        rows=await asyncio.to_thread(old_request,"GET","?noStatusUpdate=1&includeLastMessage=false")
        assert not any(row.get("name")==name for row in rows)
        journal.unlink()
        result["before"]["observer_cleanup_absence_confirmed"]=True
        after=current.FreshRoom(manifest["client_url"],credentials["users"]["lab_alice"],manifest_path.parent/"media-owned-rooms")
        new_request=after.request
        posts=0
        def lose_new(method,suffix,values=None):
            nonlocal posts
            data=new_request(method,suffix,values)
            if method=="POST" and suffix=="":
                posts+=1
                raise TimeoutError("injected after successful server create")
            return data
        after.request=lose_new
        try:
            await after.open()
            raise AssertionError("loss not injected")
        except TimeoutError:
            pass
        assert after.token is None and after.journal.exists()
        outcome=await after.close()
        assert outcome=="absence_confirmed" and posts==1 and not after.journal.exists()
        result["after"]={"lost_create_response":True,"create_requests":posts,"reconciled_by_exact_owned_name":True,
                         "cleanup":outcome,"journal_removed_after_absence":True,"requests":after.requests}
    result["result"]="pass"
    return result


async def run(args,report):
    report["negative_cases"]=await negatives()
    old=load(args.old)
    for manifest in args.manifest:
        report["real_setup_cases"].append(await actual(manifest,old))
    report["result"]="pass"


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--old",type=Path,required=True)
    parser.add_argument("--manifest",type=Path,action="append",required=True)
    parser.add_argument("--output",type=Path,required=True)
    args=parser.parse_args()
    if len(args.manifest)>2 or args.output.exists():
        raise SystemExit("at most two owned families and a new output required")
    report={"schema":1,"result":"fail","no_media":True,"probe_sha256":sha(__file__),
            "old_harness_sha256":sha(args.old),"new_harness_sha256":sha(current.__file__),
            "fault":"discard successful create response before caller receives token",
            "original_latency_cause_established":False,"real_setup_cases":[]}
    try:
        asyncio.run(run(args,report))
    except Exception as error:
        report["failure_type"]=type(error).__name__
    args.output.write_text(json.dumps(report,indent=2)+"\n")
    print(json.dumps({"result":report["result"],"families":len(report["real_setup_cases"])}))
    raise SystemExit(0 if report["result"]=="pass" else 1)


if __name__=="__main__":
    main()
