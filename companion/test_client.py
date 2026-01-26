#!/usr/bin/env python3
import socket
import json
import sys

def test_client():
    print("Connecting to BLE Tracker Companion...")
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    
    try:
        sock.connect(('127.0.0.1', 9999))
        print("Connected! Waiting for data...")
        
        buffer = ""
        while True:
            data = sock.recv(1024).decode('utf-8')
            if not data:
                print("Connection closed by server")
                break
            
            buffer += data
            while '\n' in buffer:
                line, buffer = buffer.split('\n', 1)
                if line.strip():
                    try:
                        obj = json.loads(line)
                        print(f"Received: {json.dumps(obj, indent=2)}")
                    except json.JSONDecodeError:
                        print(f"Raw: {line}")
    
    except KeyboardInterrupt:
        print("\nDisconnecting...")
    except Exception as e:
        print(f"Error: {e}")
    finally:
        sock.close()

if __name__ == "__main__":
    test_client()
