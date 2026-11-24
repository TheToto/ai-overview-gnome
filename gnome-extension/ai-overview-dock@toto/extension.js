import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import { Extension } from 'resource:///org/gnome/shell/extensions/extension.js';

const TARGET_CLASS = 'io.github.toto.AiOverviewDesktop';
const TOP_MARGIN = 6;

export default class AiOverviewDockExtension extends Extension {
    enable() {
        this._windowSignals = new Map();
        this._createdId = global.display.connect('window-created', (_display, win) => {
            this._maybeDockWindow(win);
        });
    }

    disable() {
        if (this._createdId) {
            global.display.disconnect(this._createdId);
            this._createdId = 0;
        }
        for (const [win, ids] of this._windowSignals.entries()) {
            for (const id of ids) {
                win.disconnect(id);
            }
        }
        this._windowSignals.clear();
    }

    _maybeDockWindow(win) {
        const wmClass = win.get_wm_class() || '';
        const wmInstance = win.get_wm_class_instance() || '';
        if (
            !wmClass.includes(TARGET_CLASS) &&
            !wmInstance.includes('ai-overview-desktop')
        ) {
            return;
        }

        const position = () => {
            const monitorIdx = win.get_monitor();
            const workArea = Main.layoutManager.getWorkAreaForMonitor(monitorIdx);
            const rect = win.get_frame_rect();
            const x = Math.round(workArea.x + (workArea.width - rect.width) / 2);
            const y = workArea.y + TOP_MARGIN;
            win.move_frame(false, x, y);
            win.make_above();
        };

        const ids = [
            win.connect('first-frame', position),
            win.connect('size-changed', position),
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
