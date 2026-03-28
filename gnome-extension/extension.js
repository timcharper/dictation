import St from 'gi://St';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Shell from 'gi://Shell';
import Meta from 'gi://Meta';
import { Extension } from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';

const APP_ID = 'org.gnome.dictation';

export default class DictationExtension extends Extension {
    enable() {
        // 1. Create Tray Icon
        this._indicator = new PanelMenu.Button(0.0, 'Dictation Indicator', false);
        
        let icon = new St.Icon({
            icon_name: 'audio-input-microphone-symbolic',
            style_class: 'system-status-icon',
        });
        this._indicator.add_child(icon);

        // Add settings menu item
        let settingsItem = new PopupMenu.PopupMenuItem('Settings');
        settingsItem.connect('activate', () => {
            let appSys = Shell.AppSystem.get_default();
            let app = appSys.lookup_app(`${APP_ID}.desktop`);
            if (app) {
                app.activate();
            } else {
                // Fallback to launching the binary directly
                try {
                    let proc = Gio.Subprocess.new(['dictation'], Gio.SubprocessFlags.NONE);
                } catch (e) {
                    console.error('Failed to launch dictation app:', e);
                }
            }
        });
        this._indicator.menu.addMenuItem(settingsItem);

        Main.panel.addToStatusArea(this.uuid, this._indicator);

        // TODO: Setup DBus interface to communicate with the Rust app.
        // The Rust app will be able to request keyboard focus and start/stop recording events
        // via this DBus interface.
        console.log(`[${this.uuid}] Enabled`);
    }

    disable() {
        if (this._indicator) {
            this._indicator.destroy();
            this._indicator = null;
        }

        console.log(`[${this.uuid}] Disabled`);
    }
}
