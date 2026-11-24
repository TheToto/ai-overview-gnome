import GLib from 'gi://GLib';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import { Extension } from 'resource:///org/gnome/shell/extensions/extension.js';

const TARGET_CLASS = 'io.github.toto.AiOverviewDesktop';
const TOP_MARGIN = 6;

export default class AiOverviewDockExtension extends Extension {
    enable() {
        this._windowSignals = new Map();
        this._timerIds = new Set();

        log('[ai-overview-dock] Extension enabled');

        this._mapId = global.window_manager.connect('map', (_wm, actor) => {
            this._maybeDockWindow(actor?.meta_window);
        });
        this._createdId = global.display.connect('window-created', (_display, win) => {
            this._maybeDockWindow(win);
        });

        for (const actor of global.get_window_actors()) {
            this._maybeDockWindow(actor.meta_window);
        }
    }

    disable() {
        log('[ai-overview-dock] Extension disabled');

        if (this._mapId) {
            global.window_manager.disconnect(this._mapId);
            this._mapId = 0;
        }
        if (this._createdId) {
            global.display.disconnect(this._createdId);
            this._createdId = 0;
        }
        for (const sourceId of this._timerIds) {
            GLib.Source.remove(sourceId);
        }
        this._timerIds.clear();

        for (const [win, ids] of this._windowSignals.entries()) {
            for (const id of ids) {
                win.disconnect(id);
            }
        }
        this._windowSignals.clear();
    }

    _isTargetWindow(win) {
        if (!win) {
            return false;
        }
        const appId = win.get_gtk_application_id?.() || '';
        const wmClass = win.get_wm_class() || '';
        const wmInstance = win.get_wm_class_instance() || '';
        return (
            appId.includes(TARGET_CLASS) ||
            wmClass.includes(TARGET_CLASS) ||
            wmInstance.includes('ai-overview-desktop')
        );
    }

    _maybeDockWindow(win) {
        if (!this._isTargetWindow(win) || this._windowSignals.has(win)) {
            return;
        }

        const position = (reason) => {
            const monitorIdx = win.get_monitor();
            const workArea = Main.layoutManager.getWorkAreaForMonitor(monitorIdx);
            const rect = win.get_frame_rect();
            if (rect.width <= 0) {
                return;
            }
            const x = Math.round(workArea.x + (workArea.width - rect.width) / 2);
            const y = workArea.y + TOP_MARGIN;
            win.move_frame(true, x, y);
            const after = win.get_frame_rect();
            log(
                `[ai-overview-dock] (${reason}) monitor=${monitorIdx} workArea=${workArea.x},${workArea.y} ${workArea.width}x${workArea.height} before=${rect.x},${rect.y} target=${x},${y} after=${after.x},${after.y}`
            );
        };

        position('init');
        const idleId = GLib.idle_add(GLib.PRIORITY_DEFAULT_IDLE, () => {
            this._timerIds.delete(idleId);
            position('idle');
            return GLib.SOURCE_REMOVE;
        });
        this._timerIds.add(idleId);

        const timeoutId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 80, () => {
            this._timerIds.delete(timeoutId);
            position('timeout80');
            return GLib.SOURCE_REMOVE;
        });
        this._timerIds.add(timeoutId);

        const ids = [
            win.connect('shown', () => position('shown')),
            win.connect('size-changed', () => position('size-changed')),
        ];
        ids.push(
            win.connect('unmanaging', () => {
                for (const id of ids) {
                    win.disconnect(id);
                }
                this._windowSignals.delete(win);
            })
        );
        this._windowSignals.set(win, ids);
    }
}
