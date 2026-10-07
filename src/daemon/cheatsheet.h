// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include "boardprofile.h"
#include "engine.h"
#include "kdenliveclient.h"

#include <QJsonObject>
#include <QObject>
#include <functional>

class QTimer;

namespace cs {

// What every key and knob does right now, for an on-screen overlay that the
// desktop renders (smplOS: eww). Follows the active profile, Kdenlive's context
// layers and daemon modes; the daemon only supplies content and visibility.
//
// Content (content()):
//   {visible, title, profile, layers[], window{class,title}, context{focus}?,
//    notice, options{opacity, autoHideMs, position},
//    layout{id, name, rows, columns, source},
//    keys:  [{control, row, column, <entry>}],
//    knobs: [{control, row, column, ccw, press, cw, shiftCcw, shiftCw}]}
// where <entry> / each knob field is
//   {bound, label, kind, custom, state, binding, profile, layer, active}
// (unbound: bound false, label "", kind "none").
class Cheatsheet : public QObject
{
    Q_OBJECT
public:
    Cheatsheet(Engine *engine, KdenliveClient *kd, QObject *parent = nullptr);

    void setLayoutProvider(std::function<BoardProfile()> f);
    QJsonObject content() const;
    // The same for any window class and (optionally) Kdenlive context, from the
    // running config, without changing anything: for editors and previews.
    // Kdenlive is assumed to answer, so Kdenlive bindings show as they would work.
    QJsonObject previewFor(const QString &windowClass, const QString &title, const QVariantMap &kdenliveContext) const;
    static QJsonObject preview(const Config &cfg, const BoardProfile &layout, const QString &windowClass, const QString &title,
                               const QVariantMap &kdenliveContext);

    bool isVisible() const { return m_visible; }
    void show(bool byHold = false);
    void hide();
    void toggle();
    void request(const QString &op);  // toggle | show | hide (from the engine)
    void noteInput();                 // pad input: restarts the auto-hide timer
    void invalidate();                // something the content depends on changed

Q_SIGNALS:
    void visibilityChanged(bool visible);
    // While visible: once when shown, then whenever the content changes.
    void changed(const QJsonObject &content);

private:
    QJsonObject entry(const std::optional<Engine::Resolution> &r) const;
    bool isActive(const Binding &b) const;
    void refresh();

    Engine *m_engine;
    KdenliveClient *m_kd;
    std::function<BoardProfile()> m_layout;
    bool m_visible = false;
    QTimer *m_debounce;
    QTimer *m_autoHide;
    QByteArray m_last;
};

} // namespace cs
