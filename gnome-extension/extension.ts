import St from 'gi://St';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Gdk from 'gi://Gdk';
import Shell from 'gi://Shell';
import Meta from 'gi://Meta';
import Clutter from 'gi://Clutter';
import { Extension } from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';

const APP_ID = 'org.gnome.dictation';
const BUS_NAME = 'org.gnome.dictation.Extension';
const OBJECT_PATH = '/org/gnome/dictation/Extension';

const DictationInterface = `
<node>
  <interface name="org.gnome.dictation.Extension">
    <method name="Update">
      <arg type="s" name="icon_name" direction="in"/>
      <arg type="a(ss)" name="menu_items" direction="in"/>
    </method>
    <method name="RaiseApp"/>
    <method name="GetClipboard">
      <arg type="s" name="text" direction="out"/>
    </method>
    <method name="SetClipboard">
      <arg type="s" name="text" direction="in"/>
    </method>
    <method name="TypeString">
      <arg type="s" name="text" direction="in"/>
    </method>
    <method name="RegisterShortcut">
      <arg type="s" name="shortcut" direction="in"/>
    </method>
    <signal name="MenuItemSelected">
      <arg type="s" name="id"/>
    </signal>
    <signal name="ShortcutPressed"/>
  </interface>
</node>`;

export default class DictationExtension extends Extension {
    private _indicator: PanelMenu.Button | null = null;
    private _dbusImpl: any = null;
    private _virtualKeyboard: Clutter.VirtualInputDevice | null = null;
    private _currentShortcut: string | null = null;
    private _ownNameId: number = 0;

    enable() {
        this._indicator = null;
        this._dbusImpl = null;
        this._virtualKeyboard = null;
        this._currentShortcut = null;
        this._ownNameId = 0;

        // 1. Setup DBus
        this._dbusImpl = Gio.DBusExportedObject.wrapJSObject(DictationInterface, this);
        this._dbusImpl.export(Gio.DBus.session, OBJECT_PATH);

        // Explicitly own the bus name
        this._ownNameId = Gio.bus_own_name(
            Gio.BusType.SESSION,
            BUS_NAME,
            Gio.BusNameOwnerFlags.NONE,
            (connection, name) => {
                console.log(`[${this.uuid}] Gained bus name: ${name}`);
            },
            (connection, name) => {
                console.log(`[${this.uuid}] Lost bus name: ${name}`);
            },
            () => {
                console.log(`[${this.uuid}] Bus name ${BUS_NAME} disappeared`);
            }
        );

        // 2. Initial Indicator
        this._createIndicator('audio-input-microphone-symbolic', []);

        console.log(`[${this.uuid}] Enabled`);
    }

    disable() {
        this._unregisterShortcut();

        if (this._ownNameId) {
            Gio.bus_unown_name(this._ownNameId);
            this._ownNameId = 0;
        }

        if (this._dbusImpl) {
            this._dbusImpl.unexport();
            this._dbusImpl = null;
        }

        if (this._indicator) {
            this._indicator.destroy();
            this._indicator = null;
        }

        this._virtualKeyboard = null;

        console.log(`[${this.uuid}] Disabled`);
    }

    // --- DBus Methods ---

    Update(iconName: string, menuItems: [string, string][]) {
        this._createIndicator(iconName, menuItems);
    }

    RaiseApp() {
        let app = Shell.AppSystem.get_default().lookup_app(`${APP_ID}.desktop`);
        if (app) {
            app.activate();
        } else {
            // Fallback: try to find a window with the app id
            let windows = global.get_window_actors();
            for (let win of windows) {
                let metaWin = win.get_meta_window();
                if (metaWin && (metaWin.get_wm_class_instance() === APP_ID || metaWin.get_id() === APP_ID)) {
                    metaWin.activate(global.get_current_time());
                    return;
                }
            }
        }
    }

    GetClipboard(): Promise<string> {
        let clipboard = St.Clipboard.get_default();
        return new Promise((resolve) => {
            clipboard.get_text(St.ClipboardType.CLIPBOARD, (c, text) => {
                resolve(text || "");
            });
        });
    }

    SetClipboard(text: string) {
        let clipboard = St.Clipboard.get_default();
        clipboard.set_text(St.ClipboardType.CLIPBOARD, text);
    }

    TypeString(text: string) {
        if (!this._virtualKeyboard) {
            let seat = Clutter.get_default_backend().get_default_seat();
            this._virtualKeyboard = seat.create_virtual_device(Clutter.InputDeviceType.KEYBOARD_DEVICE);
        }

        for (let i = 0; i < text.length; i++) {
            let char = text.charCodeAt(i);
            // In modern GNOME, unicode_to_keyval is in Gdk
            let keyval = Gdk.unicode_to_keyval(char);
            if (keyval) {
                this._virtualKeyboard.notify_keyval(Clutter.get_current_event_time(), keyval, Clutter.KeyState.PRESSED);
                this._virtualKeyboard.notify_keyval(Clutter.get_current_event_time(), keyval, Clutter.KeyState.RELEASED);
            }
        }
    }

    RegisterShortcut(shortcut: string) {
        this._unregisterShortcut();
        this._currentShortcut = shortcut;

        let settings = this.getSettings();
        settings.set_strv('dictation-shortcut', [shortcut]);

        Main.wm.addKeybinding('dictation-shortcut',
            settings,
            Meta.KeyBindingFlags.NONE,
            Shell.ActionMode.ALL,
            () => {
                this._dbusImpl.emit_signal('ShortcutPressed', null);
            }
        );
    }

    // --- Internal Helpers ---

    private _createIndicator(iconName: string, menuItems: [string, string][]) {
        if (this._indicator) {
            this._indicator.destroy();
        }

        this._indicator = new PanelMenu.Button(0.0, 'Dictation Indicator', false);
        let icon = new St.Icon({
            icon_name: iconName || 'audio-input-microphone-symbolic',
            style_class: 'system-status-icon',
        });
        this._indicator.add_child(icon);

        menuItems.forEach(([id, label]) => {
            let item = new PopupMenu.PopupMenuItem(label);
            item.connect('activate', () => {
                this._dbusImpl.emit_signal('MenuItemSelected', GLib.Variant.new('(s)', [id]));
            });
            this._indicator!.menu.addMenuItem(item);
        });

        Main.panel.addToStatusArea(this.uuid, this._indicator);
    }

    private _unregisterShortcut() {
        if (this._currentShortcut) {
            Main.wm.removeKeybinding('dictation-shortcut');
            this._currentShortcut = null;
        }
    }
}
