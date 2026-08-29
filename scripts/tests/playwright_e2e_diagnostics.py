#!/usr/bin/env python3
"""
playwright_e2e_diagnostics.py — Python runner for Playwright E2E suite in scripts/tests/
"""
import subprocess
import sys
import os

def main():
    print("==> Running Playwright Desktop E2E Diagnostics Suite...")
    config_path = os.path.join(os.path.dirname(__file__), "playwright.config.js")
    cmd = ["npx", "playwright", "test", "--config", config_path]
    try:
        res = subprocess.run(cmd, check=True)
        sys.exit(res.returncode)
    except Exception as e:
        print(f"Error running Playwright E2E diagnostics: {e}")
        sys.exit(1)

if __name__ == "__main__":
    main()
