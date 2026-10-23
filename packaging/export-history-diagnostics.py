#!/usr/bin/env python3
"""Export only validated history counters, never raw log lines or message data."""
import argparse
import datetime as dt
import json
from pathlib import Path
import re


LINE = re.compile(
    r"(?P<time>[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}"
    r"(?:\.[0-9]+)?(?:Z|[+-][0-9]{2}:[0-9]{2})).*?\[HISTORY\] "
    r"(?P<fields>(?:[a-z_]+=(?:[a-z_]+|-?[0-9]{1,39})\s*)+)$"
)
REQUEST_CONTEXT = re.compile(
    r"(?P<time>[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}"
    r"(?:\.[0-9]+)?(?:Z|[+-][0-9]{2}:[0-9]{2})).*?\[HISTORY_REQUEST\] "
    r"version=1 account=(?P<account>[0-9]{1,10}) capability=(?P<capability>enabled|disabled|unknown) "
    r"companion=(?P<companion>enabled|disabled|unknown) anchor=(?P<anchor>direct_pn|direct_lid|group|other|none) "
    r"empty=(?P<empty>[01]) archived=(?P<archived>[01]) active=(?P<active>[0-9]{1,10}) reserved=(?P<reserved>[0-9]{1,10})$"
)
JOB_STATUS = re.compile(
    r"(?P<time>[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}"
    r"(?:\.[0-9]+)?(?:Z|[+-][0-9]{2}:[0-9]{2})).*?\[HISTORY_JOB\] "
    r"version=1 account=(?P<account>[0-9]{1,10}) state=(?P<state>[a-z_]+) reason=(?P<reason>[a-z_]+) "
    r"failure_streak=(?P<failure_streak>[0-9]{1,3}) retry_at=(?P<retry_at>[0-9]{1,19}) "
    r"completed=(?P<completed>[0-9]{1,10}) total=(?P<total>[0-9]{1,10}) excluded=(?P<excluded>[0-9]{1,10})$"
)
MEDIA_STATUS = re.compile(
    r"(?P<time>[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}"
    r"(?:\.[0-9]+)?(?:Z|[+-][0-9]{2}:[0-9]{2})).*?\[MEDIA_JOB\] "
    r"version=1 account=(?P<account>[0-9]{1,10}) snapshot_at=(?P<snapshot_at>[0-9]{1,19}) "
    r"enabled=(?P<enabled>[01]) user_paused=(?P<user_paused>[01]) privacy_paused=(?P<privacy_paused>[01]) "
    r"connected=(?P<connected>[01]) runnable=(?P<runnable>[01]) phone_runnable=(?P<phone_runnable>[01]) "
    r"phone_recovery=(?P<phone_recovery>[01]) large_files=(?P<large_files>[01]) "
    r"target_since=(?P<target_since>-?[0-9]{1,19}) cdn_pending=(?P<cdn_pending>[0-9]{1,20}) "
    r"needs_phone=(?P<needs_phone>[0-9]{1,20}) downloaded=(?P<downloaded>[0-9]{1,20}) "
    r"unavailable=(?P<unavailable>[0-9]{1,20}) policy_too_large=(?P<policy_too_large>[0-9]{1,20}) "
    r"cancelled=(?P<cancelled>[0-9]{1,20}) schedule_known=(?P<schedule_known>[01]) "
    r"cdn_due=(?P<cdn_due>[0-9]{1,20}) "
    r"phone_due=(?P<phone_due>[0-9]{1,20}) active=(?P<active>[0-9]{1,20}) "
    r"retry_at=(?P<retry_at>[0-9]{1,19}) phone_quota_retry_at=(?P<phone_quota_retry_at>-1|[0-9]{1,19}) "
    r"bytes=(?P<bytes>-1|[0-9]{1,20}) "
    r"budget_mib=(?P<budget_mib>[0-9]{1,10}) budget_reached=(?P<budget_reached>[01])$"
)
HISTORY_ROWS = re.compile(
    r"(?P<time>[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}"
    r"(?:\.[0-9]+)?(?:Z|[+-][0-9]{2}:[0-9]{2})).*?\[HISTORY_ROWS\] "
    r"version=1 account=(?P<account>[0-9]{1,10}) snapshot_at=(?P<snapshot_at>[0-9]{1,19}) "
    r"added=(?P<added>[0-9]{1,20})$"
)
NUMBERS = {"version", "account", "sync_type", "percent", "chats", "entries", "elapsed_ms"}
STAGES = {"received", "stored", "already_stored", "parse_failed", "store_failed", "quiet",
          "dispatch_started", "dispatch_timeout", "request_rejected_not_connected",
          "request_rejected_invalid_chat", "request_deferred", "request_sent",
          "request_failed", "request_timeout"}
FLOWS = {"initial", "on_demand", "replay"}


def extract(line):
    match = LINE.search(line.rstrip("\r\n"))
    if not match:
        return None
    fields = {}
    for token in match["fields"].split():
        key, value = token.split("=", 1)
        if key in fields:
            return None
        fields[key] = value
    if set(fields) != NUMBERS | {"flow", "stage"}:
        return None
    if fields["flow"] not in FLOWS or fields["stage"] not in STAGES:
        return None
    try:
        stamp = dt.datetime.fromisoformat(match["time"].replace("Z", "+00:00"))
        for key in NUMBERS:
            fields[key] = int(fields[key])
    except ValueError:
        return None
    if stamp.tzinfo is None or fields["version"] != 1:
        return None
    if not 0 <= fields["account"] <= 2**32 - 1:
        return None
    if not -1 <= fields["sync_type"] <= 2**31 - 1 or not -1 <= fields["percent"] <= 100:
        return None
    if any(not 0 <= fields[key] <= 2**64 - 1 for key in ("chats", "entries")):
        return None
    if not 0 <= fields["elapsed_ms"] <= 2**128 - 1:
        return None
    if fields["stage"] == "quiet" and fields["flow"] != "initial":
        return None
    if (fields["stage"].startswith("request_") or fields["stage"].startswith("dispatch_")) \
            and fields["flow"] != "on_demand":
        return None
    return {"time": match["time"], **fields}


def extract_request_context(line):
    match = REQUEST_CONTEXT.search(line.rstrip("\r\n"))
    if not match:
        return None
    record = match.groupdict()
    try:
        dt.datetime.fromisoformat(record["time"].replace("Z", "+00:00"))
        for key in ("account", "empty", "archived", "active", "reserved"):
            record[key] = int(record[key])
    except ValueError:
        return None
    if not 0 <= record["account"] <= 2**32 - 1:
        return None
    return record


def extract_job_status(line):
    match = JOB_STATUS.search(line.rstrip("\r\n"))
    if not match:
        return None
    record = match.groupdict()
    states = {"idle", "initial_sync", "running", "backing_off", "paused", "needs_attention", "finished"}
    reasons = {"none", "queued", "waiting_for_phone", "advancing", "retry_send", "retry_timeout",
               "retry_no_progress", "retry_parse", "retry_storage", "waiting_for_anchor", "reached_cutoff",
               "phone_exhausted", "excluded_locked", "excluded_left", "excluded_unsupported",
               "correlation_unavailable", "retry_limit_reached"}
    if record["state"] not in states or record["reason"] not in reasons:
        return None
    try:
        dt.datetime.fromisoformat(record["time"].replace("Z", "+00:00"))
        for key in ("account", "failure_streak", "retry_at", "completed", "total", "excluded"):
            record[key] = int(record[key])
    except ValueError:
        return None
    if not 0 <= record["account"] <= 2**32 - 1 or not 0 <= record["failure_streak"] <= 255:
        return None
    return record


def extract_media_status(line):
    match = MEDIA_STATUS.search(line.rstrip("\r\n"))
    if not match:
        return None
    record = match.groupdict()
    try:
        dt.datetime.fromisoformat(record["time"].replace("Z", "+00:00"))
        for key in ("account", "snapshot_at", "enabled", "user_paused", "privacy_paused",
                    "connected", "runnable", "phone_runnable", "phone_recovery", "large_files",
                    "target_since", "cdn_pending",
                    "needs_phone", "downloaded", "unavailable", "policy_too_large", "cancelled",
                    "schedule_known",
                    "cdn_due", "phone_due", "active", "retry_at", "phone_quota_retry_at",
                    "bytes", "budget_mib", "budget_reached"):
            record[key] = int(record[key])
    except ValueError:
        return None
    if not 0 <= record["account"] <= 2**32 - 1 or record["snapshot_at"] < 0:
        return None
    if not -(2**63) <= record["target_since"] <= 2**63 - 1 or record["bytes"] < -1:
        return None
    for key in ("cdn_pending", "needs_phone", "downloaded", "unavailable", "policy_too_large",
                "cancelled", "cdn_due", "phone_due", "active"):
        if not 0 <= record[key] <= 2**64 - 1:
            return None
    if record["retry_at"] < 0 or record["phone_quota_retry_at"] < -1:
        return None
    if not 0 <= record["budget_mib"] <= 2**32 - 1:
        return None
    return record


def extract_history_rows(line):
    match = HISTORY_ROWS.search(line.rstrip("\r\n"))
    if not match:
        return None
    record = match.groupdict()
    try:
        dt.datetime.fromisoformat(record["time"].replace("Z", "+00:00"))
        for key in ("account", "snapshot_at", "added"):
            record[key] = int(record[key])
    except ValueError:
        return None
    if not 0 <= record["account"] <= 2**32 - 1 or record["snapshot_at"] < 0:
        return None
    if not 0 <= record["added"] <= 2**64 - 1:
        return None
    return record


def summarize(records):
    summary = {}
    for record in records:
        key = f'{record["account"]}:{record["flow"]}'
        group = summary.setdefault(key, {
            "account": record["account"], "flow": record["flow"],
            "events": {}, "last_event_times": {}, "stored_entries_including_repeats": 0,
        })
        stage = record["stage"]
        group["events"][stage] = group["events"].get(stage, 0) + 1
        group["last_event_times"][stage] = record["time"]
        if stage == "stored":
            group["stored_entries_including_repeats"] += record["entries"]
        group["last_observed"] = record["time"]
    return list(summary.values())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("log", type=Path)
    parser.add_argument("--since", required=True, help="ISO timestamp including timezone")
    parser.add_argument("--account", type=int, help="Local account folder number")
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    try:
        since = dt.datetime.fromisoformat(args.since.replace("Z", "+00:00"))
    except ValueError:
        parser.error("--since must be an ISO timestamp including timezone")
    if since.tzinfo is None:
        parser.error("--since must include a timezone")
    if args.account is not None and not 0 <= args.account <= 2**32 - 1:
        parser.error("--account must be a local account number from 0 through 4294967295")
    records = []
    request_contexts = []
    job_statuses = []
    media_statuses = []
    history_rows = []
    with args.log.open(encoding="utf-8", errors="replace") as stream:
        for line in stream:
            context = extract_request_context(line)
            job = extract_job_status(line)
            media = extract_media_status(line)
            rows = extract_history_rows(line)
            record = extract(line)
            if record and dt.datetime.fromisoformat(record["time"].replace("Z", "+00:00")) >= since \
                    and (args.account is None or record["account"] == args.account):
                records.append(record)
            if context and dt.datetime.fromisoformat(context["time"].replace("Z", "+00:00")) >= since \
                    and (args.account is None or context["account"] == args.account):
                request_contexts.append(context)
            if job and dt.datetime.fromisoformat(job["time"].replace("Z", "+00:00")) >= since \
                    and (args.account is None or job["account"] == args.account):
                job_statuses.append(job)
            if media and dt.datetime.fromisoformat(media["time"].replace("Z", "+00:00")) >= since \
                    and (args.account is None or media["account"] == args.account):
                media_statuses.append(media)
            if rows and dt.datetime.fromisoformat(rows["time"].replace("Z", "+00:00")) >= since \
                    and (args.account is None or rows["account"] == args.account):
                history_rows.append(rows)
    args.output.write_text(json.dumps({"records": records, "request_contexts": request_contexts,
                                      "job_statuses": job_statuses, "media_statuses": media_statuses,
                                      "history_rows": history_rows,
                                      "summary": summarize(records)}, indent=2) + "\n", encoding="utf-8")
    print(f"Exported {len(records)} history counter records.")


if __name__ == "__main__":
    main()
