#!/bin/bash

# Configuration
UUID="dictation@timharper"
SANDBOX_DIR="/tmp/gnome-sandbox-dictation"
BUS_FILE="/tmp/nested_dbus_address"
PID_FILE="/tmp/nested_gnome.pid"
LOG_FILE="/tmp/nested_gnome.log"
EXT_DIR="$(pwd)/gnome-extension"

start() {
    # Check if it's already running
    if [ -f "$PID_FILE" ] && kill -0 $(cat "$PID_FILE") 2>/dev/null; then
        echo "Nested GNOME Shell is already running (PID: $(cat $PID_FILE))."
        exit 1
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

    echo "Starting isolated nested GNOME Shell in the background..."
    
    # Run the session in the background
    # We use the same bwrap + dbus-run-session logic as test-sandbox.sh
    nohup dbus-run-session -- bwrap \
      --bind / / \
      --dev-bind /dev /dev \
      --bind /run /run \
      --bind /tmp /tmp \
      --ro-bind /sys /sys \
      --ro-bind /proc /proc \
      --tmpfs /dev/dri \
      sh -c "
        # --- ISOLATION ENVIRONMENT VARIABLES ---
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

        # Save the isolated D-Bus address
        echo \$DBUS_SESSION_BUS_ADDRESS > $BUS_FILE

        # Launch GNOME Shell
        gnome-shell --devkit --wayland &
        SHELL_PID=\$!
        
        # Give it a few seconds to boot
        sleep 8
        
        # Enable the extension
        gnome-extensions enable $UUID
        
        wait \$SHELL_PID
    " > "$LOG_FILE" 2>&1 &
    
    DBUS_PID=$!
    echo $DBUS_PID > "$PID_FILE"
    
    echo "-----------------------------------"
    echo "Started successfully! (PID: $DBUS_PID)"
    echo "D-Bus Address File : $BUS_FILE"
    echo "Live Logs          : tail -f $LOG_FILE"
    echo "-----------------------------------"
}

stop() {
    if [ -f "$PID_FILE" ]; then
        PID=$(cat "$PID_FILE")
        echo "Stopping nested GNOME Shell (PID: $PID)..."
        kill -TERM "$PID" 2>/dev/null || true
        rm -f "$PID_FILE" "$BUS_FILE"
        # Optional: rm -rf "$SANDBOX_DIR"
        echo "Stopped."
    else
        echo "No running instance found."
        rm -f "$PID_FILE" "$BUS_FILE"
    fi
}

status() {
    if [ -f "$PID_FILE" ] && kill -0 $(cat "$PID_FILE") 2>/dev/null; then
        echo "Nested GNOME Shell is RUNNING (PID: $(cat $PID_FILE))."
        if [ -f "$BUS_FILE" ]; then
            echo "D-Bus Address: $(cat $BUS_FILE)"
        fi
    else
        echo "Nested GNOME Shell is STOPPED."
    fi
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
