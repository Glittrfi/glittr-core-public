#!/bin/bash

while true
do 
    bitcoin-cli -rpcpassword=password -rpcuser=user -chain=regtest -generate 1 1000
    sleep 10
done