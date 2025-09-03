# Binance.US BTC/USDT Orderbook Data Collection

A simple AWS Lambda function that collects Bitcoin orderbook data from Binance.US WebSocket streams and stores it in S3 as Avro files.

## What This Actually Does

- **Data Source**: Binance.US WebSocket stream (`wss://stream.binance.us:9443/ws/btcusdt@depth20@100ms`)
- **Schedule**: Runs every 1 minute (configurable in template.yaml)
- **Processing**: Takes top 20 bid/ask levels and normalizes to 5 depth levels (0.01%, 0.05%, 0.1%, 0.5%, 1%)
- **Storage**: Stores as Avro files in S3 with date-based partitioning
- **Recovery**: Basic recovery function stub for backfilling gaps (incomplete)

## Architecture

```
┌─────────────────┐    WebSocket     ┌──────────────┐    Avro     ┌─────────────┐
│   Binance.US    │ ───────────────▶ │    Lambda    │ ──────────▶ │     S3      │
│  depth20@100ms  │                  │  (1 min)     │             │  Partitioned│
└─────────────────┘                  └──────────────┘             └─────────────┘
```

### Data Flow
1. Lambda triggered every minute by EventBridge
2. Connects to Binance.US WebSocket stream
3. Processes depth snapshots with top 20 bid/ask levels
4. Normalizes to 5 fixed depth levels from mid-price
5. Calculates spread, mid-price, and imbalance ratio
6. Serializes to Avro format and stores in S3

### S3 Storage Structure
```
s3://bucket-name/
  └── orderbook/
      └── year=2025/
          └── month=09/
              └── day=03/
                  └── hour=14/
                      ├── 1725379686983.avro
                      ├── 1725379746124.avro
                      └── ...
```

## Actual Avro Schema

```json
{
  "type": "record",
  "name": "OrderBook",
  "fields": [
    {"name": "timestamp_ms", "type": "long"},
    {"name": "bids", "type": {"type": "array", "items": {"type": "array", "items": "double"}}},
    {"name": "asks", "type": {"type": "array", "items": {"type": "array", "items": "double"}}},
    {"name": "spread", "type": "double"},
    {"name": "mid_price", "type": "double"},
    {"name": "imbalance_ratio", "type": "double"}
  ]
}
```

## Deployment

### Prerequisites
- AWS CLI configured
- SAM CLI installed  
- Cargo Lambda installed (`cargo install cargo-lambda`)

### Deploy
```bash
# Build both Lambda functions
cargo lambda build --release --bin orderbook-lambda
cargo lambda build --release --bin recovery

# Deploy with SAM
sam build
sam deploy --guided  # First time
sam deploy           # Subsequent deployments
```

### Manual Deploy Script
```bash
#!/bin/bash
cargo lambda build --release --bin orderbook-lambda
cargo lambda build --release --bin recovery
sam build
sam deploy
```

## Infrastructure Components

The SAM template creates:
- **S3 Bucket**: Stores Avro files with lifecycle policy (archive to Glacier after 7 days)
- **Main Lambda**: Orderbook collection function (512MB memory, 30s timeout)
- **Recovery Lambda**: Backfill function triggered by DLQ (256MB memory, 10s timeout)
- **SQS DLQ**: Dead letter queue for failed invocations
- **EventBridge Rule**: Triggers main function every 1 minute
- **CloudWatch Alarms**: Monitor WebSocket lag and Lambda failures

## Current Limitations

### Known Issues
- **Hardcoded bucket name**: "orderbook-data" is hardcoded in main.rs:88
- **No DynamoDB**: Template doesn't include DynamoDB state tracking mentioned in design
- **Basic error handling**: Simple exponential backoff, no sophisticated reconnection
- **No compression**: Files stored without Snappy compression
- **Incomplete recovery**: Recovery function is a basic stub
- **Single symbol**: Only handles BTC/USDT, not configurable

### Production Readiness Gaps
- No health checks or ping/pong for WebSocket connections
- No batching - each message triggers individual S3 write
- Missing structured logging and comprehensive monitoring
- No gap detection or proper backfill logic
- Basic CloudWatch alarms only

## Local Development

### Test WebSocket Connection
```bash
# Build and run WebSocket test
cargo build --bin test_websocket
./target/debug/test_websocket
```

### Test Full Pipeline Locally
```bash
# Runs full processing pipeline, writes to ./data/ instead of S3
cargo build --bin test_local  
./target/debug/test_local
```

### Run Lambda Locally
```bash
# Terminal 1
cargo lambda watch --bin orderbook-lambda

# Terminal 2  
cargo lambda invoke orderbook-lambda --data-file test-event.json
```

## Data Analysis

### Reading Avro Files
```python
import avro.datafile
import avro.io

with open('1725379686983.avro', 'rb') as f:
    reader = avro.datafile.DataFileReader(f, avro.io.DatumReader())
    for record in reader:
        print(f"Timestamp: {record['timestamp_ms']}")
        print(f"Mid Price: ${record['mid_price']}")
        print(f"Spread: ${record['spread']}")
```

### Athena Queries
```sql
-- Create external table
CREATE EXTERNAL TABLE orderbook_data (
  timestamp_ms bigint,
  bids array<array<double>>,
  asks array<array<double>>,
  spread double,
  mid_price double,
  imbalance_ratio double
)
STORED AS AVRO
LOCATION 's3://your-bucket/orderbook/'
PARTITIONED BY (year int, month int, day int, hour int);

-- Query recent data
SELECT 
  from_unixtime(timestamp_ms/1000) as time,
  mid_price,
  spread,
  imbalance_ratio
FROM orderbook_data
WHERE year = 2025 AND month = 9 AND day = 3
ORDER BY timestamp_ms DESC
LIMIT 100;
```

## Configuration

### Change Collection Frequency
Edit `template.yaml`:
```yaml
Schedule: rate(1 minute)  # Change to desired frequency
```

### Use Environment Variables
Update main.rs to use environment variables instead of hardcoded values:
```rust
let bucket = std::env::var("BUCKET_NAME").unwrap_or("orderbook-data".to_string());
```

## Monitoring

### View Logs
```bash
# Recent logs
sam logs -n OrderBookFunction --tail

# Stream logs  
sam logs -n OrderBookFunction --tail --follow
```

### Check S3 Data
```bash
# List recent files
aws s3 ls s3://your-bucket/orderbook/ --recursive | tail -10

# Check data for specific day
aws s3 ls s3://your-bucket/orderbook/year=2025/month=09/day=03/ --recursive --summarize
```

### Manual Invocation
```bash
aws lambda invoke \
  --function-name orderbook-lambda \
  --payload '{}' \
  response.json
```

This is a functional but basic implementation suitable for development and testing. Production use would require significant enhancements to error handling, monitoring, and data reliability.
