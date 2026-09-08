"""Regenerate the embedded LiteLLM pricing snapshot.

Fetches LiteLLM's model_prices_and_context_window.json, keeps only the
fields the cost engine needs, and writes a gzipped JSON snapshot to
crates/sivtr-core/src/usage/pricing_snapshot.json.gz.

The snapshot is committed so pricing is available without a network request;
re-run this script (and commit the result) to refresh pricing.

    python scripts/refresh-pricing.py
"""

import gzip
import json
import urllib.request
from pathlib import Path

URL = "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json"
DEST = Path(__file__).resolve().parent.parent / "crates/sivtr-core/src/usage/snapshot/pricing_snapshot.json.gz"


def main() -> None:
    """$ per token -> integer microdollars per token ($3/MTok == 3 µ$/tok)."""
    with urllib.request.urlopen(URL) as response:
        raw = json.load(response)

    snapshot = {}
    for name, entry in raw.items():
        if not isinstance(entry, dict):
            continue
        input_cost = entry.get("input_cost_per_token")
        output_cost = entry.get("output_cost_per_token")
        if input_cost is None and output_cost is None:
            continue
        record = {}
        # LiteLLM stores $ per token; we store integer microdollars per token.
        if input_cost is not None:
            record["input"] = int(round(input_cost * 1_000_000))
        if output_cost is not None:
            record["output"] = int(round(output_cost * 1_000_000))
        if entry.get("cache_read_input_token_cost") is not None:
            record["cache_read"] = int(round(entry["cache_read_input_token_cost"] * 1_000_000))
        if entry.get("cache_creation_input_token_cost") is not None:
            record["cache_creation"] = int(round(entry["cache_creation_input_token_cost"] * 1_000_000))
        snapshot[name] = record

    payload = json.dumps(
        {"source": "litellm", "models": snapshot}, separators=(",", ":"), sort_keys=True
    ).encode()
    DEST.parent.mkdir(parents=True, exist_ok=True)
    with gzip.open(DEST, "wb", compresslevel=9) as handle:
        handle.write(payload)
    print(f"wrote {DEST} ({len(snapshot)} models, {DEST.stat().st_size} bytes gz)")


if __name__ == "__main__":
    main()
