#!/bin/bash
#docker buildx build --platform linux/amd64 -t bmesh -f block-mesh-manager.Dockerfile --load .
docker run \
--platform linux/amd64 \
--network=blockmesh_network \
-e APP_ENVIRONMENT=local \
-e REDIS_URL="redis://127.0.0.1:6379" \
-e DATABASE_URL="postgres://postgres:password@localhost:5559/block-mesh" \
-e MAILGUN_SEND_KEY="" \
-t blockmesh/feature-flags-server:latest-amd64
