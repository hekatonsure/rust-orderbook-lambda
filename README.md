# realtime binance order book ingest (demo portfolio project)

this project shows a compact real-time data pipeline for ingesting, normalizing, storing, and analyzing binance btcusdt order book data. everything fits in <200 loc of rust + one cloudformation yaml.

---

## overview

- **runtime**: rust lambda (tokio), runs in 14m bursts
- **source**: binance websocket diffs (`btcusdt@depth@100ms`) + rest snapshot for bootstrap
- **normalization**: l2 aggregation to fixed depth levels (10/20/50)
- **storage**: avro + snappy in s3, partitioned by `market/symbol/dt/hour/depth`
- **resilience**: backfill/recovery mode detects gaps via `lastUpdateId` and reseeds from rest
- **monitoring**: cloudwatch embedded metrics (`ingest_lag_ms`, `drop_rate_ppm`, `reconnects`, `levels_filled`)
- **analytics**: 20-line duckdb sql on s3 for 5s volatility, spread, imbalance

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

## gotchas

* **exchange semantics**: binance diffs must be gated against `lastUpdateId`; missing sequences = drift. rest snapshot may already be stale; record both exchange `event_ts` and ingest ts.
* **lambda quirks**: 15m max runtime → run \~14m chunks; handle cold starts and reconnects with jitter. ensure idempotent s3 writes via temp keys.
* **storage**: small file tax is real. hourly partitions keep listing cheap, but compact to parquet for prod. partition pruning critical.
* **avro pitfalls**: schema evolution requires defaults. logical types must stay consistent. snappy > deflate for perf.
* **numeric traps**: float rounding at tick grid can cross sides; use integer ticks/lots internally. imbalance blows up if book thins; clip values.
* **monitoring**: drop detection can false-trip on reconnects; separate `gap_due_to_reconnect` vs real gap. lag alarms should key off p95/p99 sustained, not single spikes.
* **ddb state**: single hot partition if pk=`symbol`; shard or add `run_id`. use consistent reads. ttl old state.
* **backfill**: no public replay of diffs; demo resets from rest and tags records `recovered=true`. note data loss window.
* **ops/security**: kms on s3, least-priv iam, lifecycle rules for cost. cloudwatch metrics get \$\$; batch emit with EMF.
* **market changes**: tick/lot sizes can change. validate symbol metadata each run. log structured, sample at rate.
* **scope honesty**: this is a demo—prod would add parquet compactor, exactly-once s3 sink, multi-az failover, raw diff archive.
