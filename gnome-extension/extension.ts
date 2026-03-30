import St from 'gi://St';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Gdk from 'gi://Gdk';
import Gvc from 'gi://Gvc';
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

const DictationInterfaceXML = `
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
    <method name="UnregisterShortcut"/>
    <method name="GetVolume">
      <arg type="d" name="volume" direction="out"/>
    </method>
    <method name="SetVolume">
      <arg type="d" name="volume" direction="in"/>
    </method>
    <signal name="MenuItemSelected">
      <arg type="s" name="id"/>
    </signal>
    <signal name="ShortcutPressed"/>
  </interface>
</node>`;

export default class DictationExtension extends Extension {
    private _indicator: PanelMenu.Button | null = null;
    private _dbusId: number = 0;
    private _virtualKeyboard: Clutter.VirtualInputDevice | null = null;
    private _currentShortcut: string | null = null;
    private _ownNameId: number = 0;
    private _interfaceInfo: Gio.DBusInterfaceInfo | null = null;

    enable() {
        this._indicator = null;
        this._virtualKeyboard = null;
        this._currentShortcut = null;

        // 1. Setup DBus manually for better control over async methods
        const nodeInfo = Gio.DBusNodeInfo.new_for_xml(DictationInterfaceXML);
        this._interfaceInfo = nodeInfo.interfaces[0];

        this._dbusId = Gio.DBus.session.register_object(
            OBJECT_PATH,
            this._interfaceInfo,
            (connection, sender, objectPath, interfaceName, methodName, parameters, invocation) => {
                this._handleMethodCall(methodName, parameters, invocation);
            },
            null, // get_property
            null  // set_property
        );

        // Explicitly own the bus name
        this._ownNameId = Gio.bus_own_name(
            Gio.BusType.SESSION,
            BUS_NAME,
            Gio.BusNameOwnerFlags.NONE,
            null,
            null,
            null
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

        if (this._dbusId) {
            Gio.DBus.session.unregister_object(this._dbusId);
            this._dbusId = 0;
        }

        if (this._indicator) {
            this._indicator.destroy();
            this._indicator = null;
        }

        this._virtualKeyboard = null;

        console.log(`[${this.uuid}] Disabled`);
    }

    private _handleMethodCall(methodName: string, parameters: GLib.Variant, invocation: Gio.DBusMethodInvocation) {
        try {
            const args = parameters.deep_unpack() as any[];

            switch (methodName) {
                case 'Update':
                    this._createIndicator(args[0], args[1]);
                    invocation.return_value(null);
                    break;
                case 'RaiseApp':
                    this._raiseApp();
                    invocation.return_value(null);
                    break;
                case 'GetClipboard':
                    this._getClipboard(invocation);
                    break;
                case 'SetClipboard':
                    this._setClipboard(args[0]);
                    invocation.return_value(null);
                    break;
                case 'TypeString':
                    this._typeString(args[0]);
                    invocation.return_value(null);
                    break;
                case 'RegisterShortcut':
                    this._registerShortcut(args[0]);
                    invocation.return_value(null);
                    break;
                case 'UnregisterShortcut':
                    this._unregisterShortcut();
                    invocation.return_value(null);
                    break;
                case 'GetVolume':
                    const volume = this._getVolume();
                    invocation.return_value(GLib.Variant.new('(d)', [volume]));
                    break;
                case 'SetVolume':
                    this._setVolume(args[0]);
                    invocation.return_value(null);
                    break;
                default:
                    invocation.return_error_literal(Gio.DBusError, Gio.DBusError.UNKNOWN_METHOD, `Unknown method ${methodName}`);
            }
        } catch (e) {
            console.error(`[${this.uuid}] Error handling DBus method ${methodName}: ${e}`);
            invocation.return_error_literal(Gio.DBusError, Gio.DBusError.FAILED, `${e}`);
        }
    }

    private _getVolume(): number {
        try {
            // This works for GNOME 44+ Quick Settings
            let vol = (Main.panel.statusArea.aggregateMenu as any)?._volume?._output;
            if (!vol) vol = (Main.panel.statusArea.aggregateMenu as any)?._volume; // Fallback
            
            if (vol && vol._control) {
                const sink = vol._control.get_default_sink();
                if (sink) {
                    return sink.volume / vol._control.get_vol_max_norm();
                }
            }
        } catch (e) {
            console.error(`[${this.uuid}] Error getting volume: ${e}`);
        }
        return 1.0;
    }

    private _setVolume(volume: number) {
        try {
            let vol = (Main.panel.statusArea.aggregateMenu as any)?._volume?._output;
            if (!vol) vol = (Main.panel.statusArea.aggregateMenu as any)?._volume;
            
            if (vol && vol._control) {
                const sink = vol._control.get_default_sink();
                if (sink) {
                    sink.volume = volume * vol._control.get_vol_max_norm();
                    sink.push_volume();
                }
            }
        } catch (e) {
            console.error(`[${this.uuid}] Error setting volume: ${e}`);
        }
    }

    private _raiseApp() {
        let app = Shell.AppSystem.get_default().lookup_app(`${APP_ID}.desktop`);
        if (app) {
            app.activate();
        } else {
            let windows = global.get_window_actors();
            for (let win of windows) {
                let metaWin = win.get_meta_window();
                if (metaWin && (metaWin.get_wm_class_instance() === APP_ID || metaWin.get_id() === APP_ID)) {
                    metaWin.activate(global.get_current_time());
                    break;
                }
            }
        }
    }

    private _getClipboard(invocation: Gio.DBusMethodInvocation) {
        let clipboard = St.Clipboard.get_default();
        clipboard.get_text(St.ClipboardType.CLIPBOARD, (c, text) => {
            invocation.return_value(GLib.Variant.new('(s)', [text || ""]));
        });
    }

    private _setClipboard(text: string) {
        let clipboard = St.Clipboard.get_default();
        clipboard.set_text(St.ClipboardType.CLIPBOARD, text);
    }

    private _typeString(text: string) {
        if (!text) return;

        if (!this._virtualKeyboard) {
            let seat = Clutter.get_default_backend().get_default_seat();
            this._virtualKeyboard = seat.create_virtual_device(Clutter.InputDeviceType.KEYBOARD_DEVICE);
        }

        for (let i = 0; i < text.length; i++) {
            let char = text.charCodeAt(i);
            let keyval = Gdk.unicode_to_keyval(char);
            if (keyval) {
                this._virtualKeyboard.notify_keyval(Clutter.get_current_event_time(), keyval, Clutter.KeyState.PRESSED);
                this._virtualKeyboard.notify_keyval(Clutter.get_current_event_time(), keyval, Clutter.KeyState.RELEASED);
            }
        }
    }

    private _registerShortcut(shortcut: string) {
        this._unregisterShortcut();
        if (!shortcut) return;

        this._currentShortcut = shortcut;

        let settings = this.getSettings('org.gnome.shell.extensions.dictation');
        settings.set_strv('dictation-shortcut', [shortcut]);

        Main.wm.addKeybinding('dictation-shortcut',
            settings,
            Meta.KeyBindingFlags.NONE,
            Shell.ActionMode.ALL,
            () => {
                Gio.DBus.session.emit_signal(
                    null,
                    OBJECT_PATH,
                    BUS_NAME,
                    'ShortcutPressed',
                    null
                );
            }
        );
    }

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
                Gio.DBus.session.emit_signal(
                    null,
                    OBJECT_PATH,
                    BUS_NAME,
                    'MenuItemSelected',
                    GLib.Variant.new('(s)', [id])
                );
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
