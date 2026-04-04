#!/bin/bash

# Configuration
UUID="dictation@timcharper.com"
SANDBOX_DIR="/tmp/gnome-sandbox-dictation"
BUS_FILE="/tmp/nested_dbus_address"
PID_FILE="/tmp/nested_gnome.pid"
LOG_FILE="/tmp/nested_gnome.log"
EXT_DIR="$(pwd)/gnome-extension"

# Ensure we're in the right directory
if [ ! -d "$EXT_DIR" ]; then
    echo "Error: gnome-extension directory not found at $EXT_DIR"
    exit 1
fi

start() {
    # Check if it's already running
    if [ -f "$PID_FILE" ]; then
        PID=$(cat "$PID_FILE")
        if kill -0 "$PID" 2>/dev/null; then
            echo "Nested GNOME Shell is already running (PID: $PID)."
            exit 1
        fi
        rm -f "$PID_FILE"
    fi

    echo "Cleaning up old sandbox..."
    rm -rf "$SANDBOX_DIR"
    mkdir -p "$SANDBOX_DIR"/{config,data,state,cache}
    mkdir -p "$SANDBOX_DIR/data/gnome-shell/extensions/"

    echo "Compiling and packaging extension..."
    glib-compile-schemas "$EXT_DIR/schemas/"
    # Build the TypeScript extension
    npm run build --prefix "$EXT_DIR"
    gnome-extensions pack "$EXT_DIR" --force
    unzip -q -o "${UUID}.shell-extension.zip" -d "$SANDBOX_DIR/data/gnome-shell/extensions/${UUID}"
    rm "${UUID}.shell-extension.zip"

    # Compile schemas in the sandbox
    glib-compile-schemas "$SANDBOX_DIR/data/gnome-shell/extensions/${UUID}/schemas/"

    echo "Starting isolated nested GNOME Shell in the background..."
    rm -f "$BUS_FILE"
    
    # Run the session in the background but in its own process group
    # Using setsid ensures the whole process tree can be killed at once.
    # Using exec inside sh ensures the shell process is replaced by gnome-shell.
    nohup setsid dbus-run-session -- bwrap \
      --bind / / \
      --dev-bind /dev /dev \
      --bind /run /run \
      --bind /tmp /tmp \
      --ro-bind /sys /sys \
      --ro-bind /proc /proc \
      --tmpfs /dev/dri \
      sh -c "
        # --- ISOLATION ENVIRONMENT VARIABLES ---
        export HOME=\"$SANDBOX_DIR\"
        export XDG_CONFIG_HOME=\"$SANDBOX_DIR/config\"
        export XDG_DATA_HOME=\"$SANDBOX_DIR/data\"
        export XDG_STATE_HOME=\"$SANDBOX_DIR/state\"
        export XDG_CACHE_HOME=\"$SANDBOX_DIR/cache\"
        export XDG_DATA_DIRS=\"/usr/share:/usr/local/share\" 

        # Bypass Nvidia EGL and force software rendering
        export LIBGL_ALWAYS_SOFTWARE=1
        export __EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json

        # Ensure the agent knows where the host Wayland socket is
        export WAYLAND_DISPLAY=\${WAYLAND_DISPLAY:-wayland-0}
        export DISPLAY=$DISPLAY
        export XAUTHORITY=$XAUTHORITY

        # Save the isolated D-Bus address for the host to see
        echo \$DBUS_SESSION_BUS_ADDRESS > $BUS_FILE

        # Launch GNOME Shell
        exec gnome-shell --devkit --wayland
    " > "$LOG_FILE" 2>&1 &
    
    DBUS_PID=$!
    echo $DBUS_PID > "$PID_FILE"
    
    # Wait for the D-Bus bus to be ready
    echo -n "Waiting for D-Bus address..."
    READY=0
    for i in {1..100}; do
        if [ -s "$BUS_FILE" ]; then
            READY=1
            break
        fi
        echo -n "."
        sleep 0.1
    done
    echo ""
    
    if [ $READY -eq 0 ]; then
        echo "Error: D-Bus address file not created. Check $LOG_FILE for details."
        kill -TERM "$DBUS_PID" 2>/dev/null || true
        exit 1
    fi
    
    export DBUS_SESSION_BUS_ADDRESS=$(cat "$BUS_FILE")
    
    echo -n "Waiting for GNOME Shell to initialize..."
    if ! gdbus wait --session org.gnome.Shell --timeout 30; then
        echo "Error: GNOME Shell did not start within 30 seconds."
        tail -n 20 "$LOG_FILE"
        stop
        exit 1
    fi
    echo " Done!"
    
    # Give it one more second to settle
    sleep 1
    
    # Enable the extension
    echo "Enabling extension $UUID..."
    if ! HOME="$SANDBOX_DIR" XDG_DATA_HOME="$SANDBOX_DIR/data" XDG_CONFIG_HOME="$SANDBOX_DIR/config" gnome-extensions enable "$UUID"; then
        echo "Warning: Failed to enable extension via gnome-extensions command. Trying via D-Bus directly..."
        gdbus call --session \
                   --dest org.gnome.Shell \
                   --object-path /org/gnome/Shell \
                   --method org.gnome.Shell.Extensions.EnableExtension "$UUID"
    fi
    
    echo "-----------------------------------"
    echo "Started successfully! (PID: $DBUS_PID)"
    echo "D-Bus Address : $DBUS_SESSION_BUS_ADDRESS"
    echo "Live Logs     : tail -f $LOG_FILE"
    echo "-----------------------------------"
}

stop() {
    if [ -f "$PID_FILE" ]; then
        PID=$(cat "$PID_FILE")
        if [ -n "$PID" ] && [ "$PID" -gt 0 ] 2>/dev/null; then
            echo "Stopping nested GNOME Shell (PID: $PID) and its process group..."
            # Kill the whole process group
            kill -TERM -"$PID" 2>/dev/null || true
            sleep 2
            # Force kill if still running
            kill -9 -"$PID" 2>/dev/null || true
        fi
        rm -f "$PID_FILE" "$BUS_FILE"
        echo "Stopped."
    else
        echo "No running instance found."
        rm -f "$PID_FILE" "$BUS_FILE"
    fi
}

status() {
    if [ -f "$PID_FILE" ]; then
        PID=$(cat "$PID_FILE")
        if kill -0 "$PID" 2>/dev/null; then
            echo "Nested GNOME Shell is RUNNING (PID: $PID)."
            if [ -f "$BUS_FILE" ]; then
                echo "D-Bus Address: $(cat $BUS_FILE)"
            fi
            return 0
        fi
    fi
    echo "Nested GNOME Shell is STOPPED."
    return 1
}

# Command Router
case "$1" in
    start)
        start
        ;;
    stop)
        stop
        ;;
    restart)
        stop
        sleep 2
        start
        ;;
    status)
        status
        ;;
    *)
        echo "Usage: $0 {start|stop|restart|status}"
        exit 1
        ;;
esac
