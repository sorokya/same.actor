#!/usr/bin/env python3
"""A stand-in for the TMDB API, for local development without a real key.

Serves the fixtures in scripts/fixtures/tmdb.json at the paths the Rust service calls:

    /3/search/multi          film and tv search
    /3/search/person         people search
    /3/movie/{id}            title metadata + credits (append_to_response)
    /3/tv/{id}               title metadata + aggregate_credits
    /3/person/{id}           person metadata + combined_credits

Run it, then point the service at it:

    python3 scripts/demo-stub.py --port 4100 &
    TMDB_TOKEN=dev TMDB_BASE=http://127.0.0.1:4100/3 STATIC_DIR=dist \
        ./api-rs/target/debug/same-actor

Fixtures are public catalogue data captured from the live API; no credentials are involved.
"""

import argparse
import json
import pathlib
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

FIXTURES = pathlib.Path(__file__).parent / "fixtures" / "tmdb.json"


def load() -> dict:
    if not FIXTURES.is_file():
        sys.exit(f"missing fixtures: {FIXTURES}")
    return json.loads(FIXTURES.read_text())


class Handler(BaseHTTPRequestHandler):
    fixtures = load()

    def do_GET(self) -> None:  # noqa: N802 - http.server API
        path = self.path.split("?", 1)[0]

        if path == "/3/search/multi":
            return self.json({"page": 1, "results": self.fixtures["search_multi"]})
        if path == "/3/search/person":
            return self.json({"page": 1, "results": self.fixtures["search_person"]})

        # Discovery lists. The movie lists return titles the stub knows, so the cast-synergy
        # table has something real to show; the tv lists return ids it does not, which
        # demonstrates the empty-synergy path. `discover/*` and `trending/*` are the same
        # fixtures: the real ones differ by query, and the point here is that every list the
        # site serves renders.
        if path in (
            "/3/movie/popular",
            "/3/movie/now_playing",
            "/3/movie/upcoming",
            "/3/discover/movie",
            "/3/trending/movie/day",
        ):
            return self.json({"page": 1, "results": self.fixtures["lists_movie"]})
        if path in (
            "/3/tv/popular",
            "/3/tv/on_the_air",
            "/3/discover/tv",
            "/3/trending/tv/day",
        ):
            return self.json({"page": 1, "results": self.fixtures["lists_tv"]})

        for collection in ("media", "people"):
            entry = self.fixtures[collection].get(path)
            if entry is not None:
                return self.json(entry)

        self.json({"status_message": "The resource you requested could not be found."}, status=404)

    def json(self, payload: dict, status: int = 200) -> None:
        body = json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("content-type", "application/json; charset=utf-8")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, format: str, *args) -> None:
        sys.stderr.write(f"stub {self.address_string()} {format % args}\n")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, default=4100)
    parser.add_argument("--host", default="127.0.0.1")
    args = parser.parse_args()

    server = ThreadingHTTPServer((args.host, args.port), Handler)
    print(f"tmdb stub on http://{args.host}:{args.port}/3", file=sys.stderr)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
