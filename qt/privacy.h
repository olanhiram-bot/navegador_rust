#pragma once

#include <QMutex>
#include <QSet>
#include <QSettings>
#include <QStringList>
#include <QUrl>

class StorageExceptions {
public:
    explicit StorageExceptions(bool persistent) : persistent_(persistent) {
        if (!persistent_) return;
        for (const auto &site : QSettings().value("privacy/storageExceptions").toStringList()) {
            sites_.insert(site);
        }
    }

    static QString origin(const QUrl &url) {
        const auto scheme = url.scheme();
        if (url.host().isEmpty() || (scheme != QStringLiteral("http") && scheme != QStringLiteral("https")))
            return {};
        QUrl key;
        key.setScheme(scheme);
        key.setHost(url.host());
        const int defaultPort = scheme == QStringLiteral("https") ? 443 : 80;
        if (url.port(defaultPort) != defaultPort) key.setPort(url.port());
        return key.toString();
    }

    bool allows(const QUrl &url) const {
        const QString site = origin(url);
        QMutexLocker guard(&mutex_);
        return !site.isEmpty() && sites_.contains(site);
    }

    void set(const QUrl &url, bool allowed) {
        const QString site = origin(url);
        if (site.isEmpty()) return;
        QStringList entries;
        {
            QMutexLocker guard(&mutex_);
            if (allowed) sites_.insert(site);
            else sites_.remove(site);
            entries = sites_.values();
        }
        if (persistent_) QSettings().setValue("privacy/storageExceptions", entries);
    }

private:
    bool persistent_;
    mutable QMutex mutex_;
    QSet<QString> sites_;
};
