// SPDX-License-Identifier: GPL-2.0-or-later
#include "uinputsink.h"

#include <cerrno>
#include <cstring>
#include <fcntl.h>
#include <linux/uinput.h>
#include <sys/ioctl.h>
#include <unistd.h>

namespace cs {

UinputKeySink::UinputKeySink(QObject *parent)
    : KeySink(parent)
{
}

UinputKeySink::~UinputKeySink()
{
    close();
}

bool UinputKeySink::open(QString *error)
{
    if (m_fd >= 0) {
        return true;
    }
    const int fd = ::open("/dev/uinput", O_WRONLY | O_NONBLOCK | O_CLOEXEC);
    if (fd < 0) {
        if (error) {
            *error = QStringLiteral("open /dev/uinput: %1").arg(QString::fromLocal8Bit(std::strerror(errno)));
        }
        return false;
    }
    bool ok = ::ioctl(fd, UI_SET_EVBIT, EV_KEY) == 0 && ::ioctl(fd, UI_SET_EVBIT, EV_SYN) == 0;
    for (int code = 1; ok && code < 256; ++code) {
        // Never advertise keys that power/suspend/rfkill the machine: events for
        // unadvertised codes are dropped by the kernel.
        if (code == KEY_POWER || code == KEY_SLEEP || code == KEY_WAKEUP || code == KEY_SUSPEND || code == KEY_RFKILL
            || code == KEY_BATTERY || code == KEY_COFFEE) {
            continue;
        }
        ok = ::ioctl(fd, UI_SET_KEYBIT, code) == 0;
    }
    uinput_setup setup{};
    setup.id.bustype = BUS_VIRTUAL;
    setup.id.vendor = kUinputVendor;
    setup.id.product = kUinputProduct;
    setup.id.version = 1;
    std::strncpy(setup.name, kUinputName, UINPUT_MAX_NAME_SIZE - 1);
    ok = ok && ::ioctl(fd, UI_DEV_SETUP, &setup) == 0 && ::ioctl(fd, UI_DEV_CREATE) == 0;
    if (!ok) {
        if (error) {
            *error = QStringLiteral("uinput setup: %1").arg(QString::fromLocal8Bit(std::strerror(errno)));
        }
        ::close(fd);
        return false;
    }
    m_fd = fd;
    return true;
}

void UinputKeySink::close()
{
    if (m_fd >= 0) {
        ::ioctl(m_fd, UI_DEV_DESTROY);
        ::close(m_fd);
        m_fd = -1;
    }
    if (m_pointerFd >= 0) {
        ::ioctl(m_pointerFd, UI_DEV_DESTROY);
        ::close(m_pointerFd);
        m_pointerFd = -1;
    }
}

bool UinputKeySink::openPointer()
{
    if (m_pointerFd >= 0) {
        return true;
    }
    if (m_pointerFailed) {
        return false;
    }
    const int fd = ::open("/dev/uinput", O_WRONLY | O_NONBLOCK | O_CLOEXEC);
    bool ok = fd >= 0 && ::ioctl(fd, UI_SET_EVBIT, EV_KEY) == 0 && ::ioctl(fd, UI_SET_EVBIT, EV_REL) == 0 && ::ioctl(fd, UI_SET_EVBIT, EV_SYN) == 0;
    for (int b : {BTN_LEFT, BTN_RIGHT, BTN_MIDDLE, BTN_SIDE, BTN_EXTRA}) {
        ok = ok && ::ioctl(fd, UI_SET_KEYBIT, b) == 0;
    }
    // X and Y make it a pointer to libinput; they are never moved.
    for (int r : {REL_X, REL_Y, REL_WHEEL, REL_HWHEEL}) {
        ok = ok && ::ioctl(fd, UI_SET_RELBIT, r) == 0;
    }
    uinput_setup setup{};
    setup.id.bustype = BUS_VIRTUAL;
    setup.id.vendor = kUinputVendor;
    setup.id.product = kUinputPointerProduct;
    setup.id.version = 1;
    std::strncpy(setup.name, kUinputPointerName, UINPUT_MAX_NAME_SIZE - 1);
    ok = ok && ::ioctl(fd, UI_DEV_SETUP, &setup) == 0 && ::ioctl(fd, UI_DEV_CREATE) == 0;
    if (!ok) {
        if (fd >= 0) {
            ::close(fd);
        }
        m_pointerFailed = true;  // do not retry on every detent
        return false;
    }
    m_pointerFd = fd;
    return true;
}

void UinputKeySink::mouse(const QString &action)
{
    const auto events = mouseEvents(action);
    if (!events || m_fd < 0 || !openPointer()) {
        return;
    }
    for (const InputTriple &t : *events) {
        input_event ev{};
        ev.type = quint16(t.type);
        ev.code = quint16(t.code);
        ev.value = t.value;
        if (::write(m_pointerFd, &ev, sizeof ev) != ssize_t(sizeof ev)) {
            return;  // a full kernel buffer drops this action
        }
    }
}

namespace {
QString sysNameOf(int fd)
{
    if (fd < 0) {
        return {};
    }
    char buf[64] = {};
    if (::ioctl(fd, UI_GET_SYSNAME(sizeof buf), buf) < 0) {
        return {};
    }
    return QString::fromLatin1(buf);
}
} // namespace

QString UinputKeySink::sysName() const
{
    return sysNameOf(m_fd);
}

QString UinputKeySink::pointerSysName() const
{
    return sysNameOf(m_pointerFd);
}

void UinputKeySink::emitKey(int code, int value)
{
    input_event ev{};
    ev.type = EV_KEY;
    ev.code = quint16(code);
    ev.value = value;
    if (::write(m_fd, &ev, sizeof ev) != ssize_t(sizeof ev)) {
        // a full kernel buffer only drops this key; nothing to recover
    }
}

void UinputKeySink::sync()
{
    input_event ev{};
    ev.type = EV_SYN;
    ev.code = SYN_REPORT;
    if (::write(m_fd, &ev, sizeof ev) != ssize_t(sizeof ev)) {
    }
}

void UinputKeySink::tap(const KeyChord &chord)
{
    if (m_fd < 0 || chord.key <= 0) {
        return;
    }
    static const quint8 bits[] = {Mod::Ctrl, Mod::Shift, Mod::Alt, Mod::Meta};
    for (auto b : bits) {
        if (chord.mods & b) {
            emitKey(modifierKey(b), 1);
        }
    }
    if (chord.mods) {
        sync();
    }
    emitKey(chord.key, 1);
    sync();
    emitKey(chord.key, 0);
    sync();
    for (auto b : bits) {
        if (chord.mods & b) {
            emitKey(modifierKey(b), 0);
        }
    }
    if (chord.mods) {
        sync();
    }
}

} // namespace cs
