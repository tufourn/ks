#!/bin/sh

mkdir -p data
curl -o data/yellow_tripdata_2025-01.parquet \
	https://d37ci6vzurychx.cloudfront.net/trip-data/yellow_tripdata_2025-01.parquet
