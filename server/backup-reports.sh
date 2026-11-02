#!/usr/bin/env bash
set -euo pipefail
report_stamp=$(date -u +%Y%m%dT%H%M%SZ)
docker exec opensc_web /opensc-web --reports backup "/srv/reports/backups/bugs-${report_stamp}.sqlite3"
