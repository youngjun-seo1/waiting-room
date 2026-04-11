#!/bin/bash

echo "Stopping servers..."

# Origin (3000)
lsof -ti:3000 2>/dev/null | xargs kill 2>/dev/null && echo "  Origin (3000) stopped" || echo "  Origin (3000) not running"

# Waiting Room (8080, 8081)
lsof -ti:8080 2>/dev/null | xargs kill 2>/dev/null && echo "  Waiting Room (8080) stopped" || echo "  Waiting Room (8080) not running"
lsof -ti:8081 2>/dev/null | xargs kill 2>/dev/null && echo "  Waiting Room (8081) stopped" || echo "  Waiting Room (8081) not running"

# Admin SPA (5173)
lsof -ti:5173 2>/dev/null | xargs kill 2>/dev/null && echo "  Admin SPA (5173) stopped" || echo "  Admin SPA (5173) not running"

echo "Done."
