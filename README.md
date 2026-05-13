# Captur

S3-based screen capture server.

## Usage

```bash
cargo run -- --port 8014 --config-port 8016 --interval 3
```

### CLI Options

| Option | Default | Description |
|--------|---------|-------------|
| `--port` | 8014 | S3 server port |
| `--config-port` | 8016 | Config API port |
| `--interval` | 3 | Capture interval in seconds |
| `--key-id` | captur | S3 access key |
| `--secret-key` | captur123 | S3 secret key |
| `--storage-dir` | ./data | Storage directory |
| `--bucket` | captur | S3 bucket name |

## Web Control Panel

Open in browser:
- `http://localhost:8016/`
- `http://192.168.50.109:8016/` (device IP)

Features:
- View/change capture interval
- Start/stop capture
- Auto-refresh status every 2 seconds

## Config API

Base URL: `http://localhost:8016`

### Get/Set Interval

```bash
# Get current interval
curl http://localhost:8016/interval
# {"interval":3}

# Set interval to 10 seconds
curl -X POST http://localhost:8016/interval \
  -H "Content-Type: application/json" \
  -d '{"interval": 10}'
# {"interval":10}
```

### Start/Stop Capture

```bash
# Get capture status
curl http://localhost:8016/capture
# {"running":true}

# Stop capture
curl -X POST http://localhost:8016/capture \
  -H "Content-Type: application/json" \
  -d '{"running": false}'
# {"running":false}

# Start capture
curl -X POST http://localhost:8016/capture \
  -H "Content-Type: application/json" \
  -d '{"running": true}'
# {"running":true}
```

## S3 Access

```bash
# List buckets
aws s3 ls --endpoint-url=http://localhost:8014

# Upload file
aws s3 cp test.txt s3://captur/test.txt --endpoint-url=http://localhost:8014

# Download file
aws s3 cp s3://captur/test.txt ./test.txt --endpoint-url=http://localhost:8014
```

Use `key-id: captur` and `secret-key: captur123` for authentication.