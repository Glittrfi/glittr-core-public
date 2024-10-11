#!/bin/bash

### Prerequisities
### bitcoin-cli, bitcoind is installed on your machine

### 1. spawn local bitcoind regtest instance
mkdir -p data
cp configs/bitcoin.conf data
bitcoind -datadir=./data -regtest

### 2. spawn electrs instance

### 3. spawn btc explorer instance