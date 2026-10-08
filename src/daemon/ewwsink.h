// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include "config.h"

#include <QJsonObject>
#include <QObject>
#include <QPointer>
#include <QProcess>

class QTimer;

namespace cs {

class Cheatsheet;

// Pushes the cheatsheet into eww so the desktop needs no listener process:
// `eww update VAR=<content>` when it is shown, changes or is hidden, and, when
// a window is named, `eww open WINDOW --anchor A` after the update on show
// (reopened when the position changes) and `eww close WINDOW` before the
// update on hide. One eww call at a time, in
// order; while one runs only the latest state is kept (the sheet's own 40 ms
// debounce limits changes). A failed update skips the window's open, so a
// desktop without a running eww never gets one started.
class EwwSink : public QObject
{
    Q_OBJECT
public:
    explicit EwwSink(Cheatsheet *sheet, QObject *parent = nullptr);
    ~EwwSink() override;

    // Takes effect at once. A sheet shown under the old settings is hidden
    // there first; when enabled, eww gets the current state (hidden at start,
    // which also clears a sheet a crashed daemon left on screen).
    void setOptions(const EwwHook &o);
    const EwwHook &options() const { return m_opts; }
    // On exit: make eww show the sheet hidden, waiting at most timeoutMs.
    void finish(int timeoutMs = 3000);
    bool isBusy() const { return m_proc || !m_steps.isEmpty(); }
    // {enabled, variable, window, config, calls, failures, lastError}
    QJsonObject status() const;

    static constexpr int kCallTimeoutMs = 2000;
    static constexpr int kMaxJsonBytes = 120000;  // one argv string (Linux caps it at 128 KiB)

Q_SIGNALS:
    void message(const QString &text);  // failures, and recovery after one
    void idle();

private:
    struct Step {
        enum Kind { Update, Open, Close } kind = Update;
        QString program;
        QStringList args;
    };
    void push(const QJsonObject &content, bool visible);
    void forceHide();
    bool pending(int kind) const;
    void plan();
    void startNext();
    void done(QProcess *p, bool ok, const QString &output);
    QStringList baseArgs() const;
    static QByteArray hidden(const QByteArray &json);

    QPointer<Cheatsheet> m_sheet;
    EwwHook m_opts;
    bool m_hasWant = false;
    bool m_wantVisible = false;
    QByteArray m_wantJson;
    QString m_wantAnchor;
    // What eww was last asked for (attempted, so a failure does not loop).
    QByteArray m_sentJson;
    bool m_sentVisible = false;
    bool m_windowOpen = false;
    QString m_openAnchor;
    bool m_lastUpdateOk = true;
    // Cheatsheet::forceHide(): close and send hidden whatever eww was told.
    bool m_forceClose = false;
    bool m_forceUpdate = false;
    QList<Step> m_steps;
    QProcess *m_proc = nullptr;
    Step m_running;
    QTimer *m_timeout;
    int m_calls = 0;
    int m_failures = 0;
    QString m_lastError;
    bool m_failing = false;
};

} // namespace cs
