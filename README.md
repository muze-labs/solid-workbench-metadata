# Solid Workbench metadata

[shortcut-metadata/](shortcut-metadata/) is a Rust service for finding a public
web page's title and launcher icon. [dev/](dev/) is the shared Docker Compose
environment. Give each future service its own top-level directory and wire it
into Compose there.

## Run the service

Run `cargo run --locked --manifest-path shortcut-metadata/Cargo.toml`. The
service listens on `127.0.0.1:4175` by default. Set `METADATA_BIND` to change
the listener. Credential-free requests are allowed from any browser origin by default. Set
`WORKBENCH_ORIGIN` to restrict browser access to one exact origin.

Send JSON containing a single public HTTP(S) URL to `POST /shortcut-metadata`.
For example:

```sh
curl -s http://127.0.0.1:4175/shortcut-metadata \
  -H 'Content-Type: application/json' \
  -d '{"url":"https://www.wikipedia.org/"}'
```

The reply contains `title` and, when available, an `iconUrl` relative to the
service origin. Icons are PNG thumbnails. The bounded in-memory icon cache
can evict files and clears on restart. Clients can retain the downloaded icon
or fall back and rediscover it. `GET /healthz` returns 204.

The service reads the static page head, its advertised web manifest and a
bounded number of icon candidates. It supports PNG, JPEG, ICO, WebP and GIF
sources. A single eight-second deadline, an eight-request concurrency limit and
an instance-wide request budget bound discovery. Deployments that expose the
endpoint to multiple users can enforce caller-specific quotas at their
authenticated gateway. Treat input URLs as sensitive when configuring access
logs. SVG icons and pages that generate their metadata through JavaScript can
fall back to the hostname and default icon.

## Run the development environment

Place this repo beside the Solid Workbench checkout, or set `WORKBENCH_DIR`
to the Workbench checkout's absolute path. Run from this repository:

```sh
docker compose -f dev/compose.yaml up --build
```

Open `http://localhost:4174/workbench/`. The same origin serves the metadata
endpoint at `http://localhost:4174/shortcut-metadata`. Set `WORKBENCH_PORT` to
use another local port. Re-run with `--build` after changing either checkout.
The existing Workbench build requires a Solid identity and Pod for its usual
authenticated workflows. Launcher invocation and provider preferences belong to
the subsequent Workbench integration cycle.
