// SPDX-License-Identifier: GPL-2.0-or-later
// Virtual keyboard through /dev/uinput. The device uses its own IDs (never
// 1189:8890), so the pad discovery can never grab it.
#pragma once

#include "keysink.h"

namespace cs {

constexpr quint16 kUinputVendor = 0x1d6b;   // Linux Foundation
constexpr quint16 kUinputProduct = 0x0cf1;  // arbitrary, "control surface"
constexpr const char *kUinputName = "control-surface virtual keyboard";
constexpr quint16 kUinputPointerProduct = 0x0cf2;
constexpr const char *kUinputPointerName = "control-surface virtual pointer";

class UinputKeySink : public KeySink
{
    Q_OBJECT
public:
    explicit UinputKeySink(QObject *parent = nullptr);
    ~UinputKeySink() override;
    bool open(QString *error = nullptr);
    void close();
    bool isReady() const override { return m_fd >= 0; }
    void tap(const KeyChord &chord) override;
    // Mouse actions go to a second virtual device (buttons, wheels, X/Y), so
    // libinput treats it as a pointer; it is created on first use.
    void mouse(const QString &action) override;
    QString sysName() const;  // e.g. "input42" once created
    bool openPointer();       // create the pointer device now (tests grab it before use)
    QString pointerSysName() const;

private:
    void emitKey(int code, int value);
    void sync();
    int m_fd = -1;
    int m_pointerFd = -1;
    bool m_pointerFailed = false;
};

} // namespace cs
