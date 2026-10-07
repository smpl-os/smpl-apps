// SPDX-License-Identifier: GPL-2.0-or-later
// Virtual keyboard through /dev/uinput. The device uses its own IDs (never
// 1189:8890), so the pad discovery can never grab it.
#pragma once

#include "keysink.h"

namespace cs {

constexpr quint16 kUinputVendor = 0x1d6b;   // Linux Foundation
constexpr quint16 kUinputProduct = 0x0cf1;  // arbitrary, "control surface"
constexpr const char *kUinputName = "control-surface virtual keyboard";

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
    QString sysName() const;  // e.g. "input42" once created

private:
    void emitKey(int code, int value);
    void sync();
    int m_fd = -1;
};

} // namespace cs
