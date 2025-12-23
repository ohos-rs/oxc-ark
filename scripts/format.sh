#! /bin/bash

ROOT_DIR=$(cd $(dirname $0)/..; pwd)

cd $ROOT_DIR

start_time=$(date +%s)
cargo run -- format "/Users/ranger/Desktop/applications_app_samples/**/*.{ets,ts}" -t=15
end_time=$(date +%s)
duration=$((end_time - start_time))
echo "Time taken: $duration seconds"