WITH depth_volatility AS (
  SELECT 
    timestamp_ms/5000 as bucket_5s,
    STDDEV(mid_price)/AVG(mid_price) * 10000 as vol_bps,
    AVG(spread) as avg_spread_usd,
    AVG(imbalance_ratio) as avg_imbalance,
    PERCENTILE_CONT(0.9) WITHIN GROUP (ORDER BY spread) as p90_spread
  FROM read_parquet('s3://orderbook-data/orderbook/*/*/*/*/*.avro')
  WHERE timestamp_ms > EXTRACT(epoch FROM NOW() - INTERVAL '1 hour') * 1000
  GROUP BY 1
)
SELECT 
  FROM_UNIXTIME(bucket_5s * 5) as time,
  vol_bps,
  avg_spread_usd,
  avg_imbalance,
  p90_spread,
  vol_bps - LAG(vol_bps) OVER (ORDER BY bucket_5s) as vol_change
FROM depth_volatility 
ORDER BY bucket_5s DESC 
LIMIT 100;
