# realtime binance order book ingest (demo portfolio project)

this project shows a compact real-time data pipeline for ingesting, normalizing, storing, and analyzing binance btcusdt order book data. everything fits in <200 loc of rust + one aws sam template.

---

## overview

- **runtime**: rust lambda (tokio), runs in 14m bursts
- **source**: binance websocket diffs (`btcusdt@depth@100ms`) + rest snapshot for bootstrap
- **normalization**: l2 aggregation to fixed depth levels (10/20/50)
- **storage**: avro + snappy in s3, partitioned by `market/symbol/dt/hour/depth`
- **resilience**: backfill/recovery mode detects gaps via `lastUpdateId` and reseeds from rest
- **monitoring**: cloudwatch embedded metrics (`ingest_lag_ms`, `drop_rate_ppm`, `reconnects`, `levels_filled`)
- **analytics**: 20-line duckdb sql on s3 for 5s volatility, spread, imbalance
- **infra**: deployed with **aws sam** template (instead of raw cloudformation)

---

## schema (avro)

```json
{
  "type":"record","name":"l2",
  "fields":[
    {"name":"venue","type":"string"},
    {"name":"symbol","type":"string"},
    {"name":"event_ts","type":"long","logicalType":"timestamp-micros"},
    {"name":"ingest_ts","type":"long","logicalType":"timestamp-micros"},
    {"name":"last_update_id","type":"long"},
    {"name":"depth","type":"int"},
    {"name":"levels","type":{
      "type":"array","items":{
        "type":"record","name":"lvl",
        "fields":[
          {"name":"side","type":{"type":"enum","name":"side","symbols":["bid","ask"]}},
          {"name":"px","type":"double"},
          {"name":"qty","type":"double"}
        ]}}},
    {"name":"mid","type":"double"},
    {"name":"spread_bps","type":"double"},
    {"name":"imbalance","type":"double"},
    {"name":"gap_detected","type":"boolean","default":false}
  ]
}
```

---

## sample duckdb query

```sql
PRAGMA s3_region='us-west-2';
SET s3_url_style='path';
LOAD avro;

CREATE OR REPLACE VIEW v AS
SELECT
  event_ts::TIMESTAMP as ts,
  mid, spread_bps, imbalance
FROM read_avro('s3://bucket/market=binance/symbol=btcusdt/dt=2025-09-01/hour=*/depth=20/*.avro');

WITH b AS (
  SELECT
    window_start as t5,
    FIRST(mid) OVER (PARTITION BY window_start ORDER BY ts) AS mid0,
    LAST(mid)  OVER (PARTITION BY window_start ORDER BY ts) AS mid1,
    AVG(spread_bps) AS avg_spread_bps,
    AVG(imbalance)  AS avg_imb
  FROM v
  WINDOW tumble AS (PARTITION BY 1 ORDER BY ts RANGE INTERVAL 5 SECOND)
)
SELECT
  t5,
  CASE WHEN mid0>0 AND mid1>0 THEN
    1e4 * ABS(LN(mid1) - LN(mid0))  -- 5s vol in bps
  ELSE NULL END AS vol5s_bps,
  avg_spread_bps,
  avg_imb
FROM b
ORDER BY t5;
```

---

## aws sam template (high level)

* **resources**:

  * s3 bucket (encrypted, versioning enabled)
  * dynamodb table for state (`symbol` pk, optional `run_id`)
  * rust lambda function (zip artifact) with env vars (`BUCKET`, `DEPTHS`, etc.)
  * eventbridge rules (ingest every 14m, recover every 5m) wired to same lambda with different `MODE`
  * iam role with least-privileges (s3 put, ddb r/w, cw logs/metrics, ssm read)
  * cloudwatch alarms for lag and drop rate

* **local dev**: `sam build` (cargo lambda), `sam local invoke` for quick tests

* **deploy**: `sam deploy --guided` → creates bucket, lambda, ddb, eventbridge rules automatically

---

## gotchas

* **exchange semantics**: diffs must be gated by `lastUpdateId` or you silently drift. rest snapshot may already be stale; record both `event_ts` and ingest time.
* **lambda quirks**: 15m max → run 14m chunks, reconnect with jitter, persist seq state. writes should be idempotent via temp key then final s3 key.
* **storage**: hourly partitions avoid s3 listing pain, but parquet compaction is necessary beyond demo scale.
* **avro pitfalls**: schema evolution needs defaults, logical types must be consistent, snappy > deflate.
* **numeric traps**: tick rounding can straddle sides; better to store ints for ticks/lots. imbalance can explode—clip.
* **monitoring**: drop detection can false-trip on reconnects; separate reconnect gaps vs real stream loss. alarm on sustained p95 lag not single blips.
* **ddb state**: pk hot-spotting if single symbol; shard or add `run_id`. enforce consistent reads; ttl cleanup.
* **backfill**: no public diff replay → demo reseeds from rest and tags `recovered=true`. data loss window is explicit.
* **ops/security**: kms on s3, least-priv iam, bucket lifecycle mgmt. cw metrics \$\$ → batch emit.
* **market changes**: tick/lot sizes can change—validate symbol metadata each run. log structured, sample if high volume.
* **scope honesty**: demo only. prod would add parquet compactor, exactly-once s3 sink, multi-az, raw diff lane.
