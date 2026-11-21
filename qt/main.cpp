// Shell Qt WebEngine: vista nativa de Chromium (sin screencast), pensada para
// sustituir el front-end de GPUI cuando se integre el motor auditable por CDP.

#include <QAction>
#include <QApplication>
#include <QCloseEvent>
#include <QDir>
#include <QFileInfo>
#include <QKeySequence>
#include <QLineEdit>
#include <QMainWindow>
#include <QShortcut>
#include <QStandardPaths>
#include <QStatusBar>
#include <QTabWidget>
#include <QToolBar>
#include <QSizePolicy>
#include <QUrl>
#include <QWebEngineDownloadRequest>
#include <QWebEngineProfile>
#include <QWebEngineView>

class Shell : public QMainWindow {
public:
    explicit Shell(const QUrl &initial) {
        setWindowTitle(QStringLiteral("Navegador"));
        resize(1100, 780);

        auto *toolbar = addToolBar(QStringLiteral("Navegación"));
        toolbar->setMovable(false);
        auto *back = toolbar->addAction(QStringLiteral("Atrás"));
        auto *forward = toolbar->addAction(QStringLiteral("Adelante"));
        auto *reload = toolbar->addAction(QStringLiteral("Recargar"));
        address_ = new QLineEdit(this);
        address_->setClearButtonEnabled(true);
        address_->setPlaceholderText(QStringLiteral("Busca o escribe una dirección"));
        address_->setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Preferred);
        address_->setMinimumWidth(320);
        toolbar->addWidget(address_);

        tabs_ = new QTabWidget(this);
        tabs_->setTabsClosable(true);
        tabs_->setMovable(true);
        setCentralWidget(tabs_);

        connect(back, &QAction::triggered, this, [this] { view()->back(); });
        connect(forward, &QAction::triggered, this, [this] { view()->forward(); });
        connect(reload, &QAction::triggered, this, [this] { view()->reload(); });
        connect(address_, &QLineEdit::returnPressed, this, [this] { navigate(address_->text()); });
        connect(tabs_, &QTabWidget::currentChanged, this, [this](int) { syncAddress(); });
        connect(tabs_, &QTabWidget::tabCloseRequested, this, &Shell::closeTab);

        auto shortcut = [this](const QKeySequence &key, auto action) {
            auto *shortcut = new QShortcut(key, this);
            connect(shortcut, &QShortcut::activated, this, action);
        };
        shortcut(QKeySequence::New, [this] { newTab(QUrl()); });
        shortcut(QKeySequence::Close, [this] { closeTab(tabs_->currentIndex()); });
        shortcut(QKeySequence::Refresh, [this] { view()->reload(); });
        shortcut(QKeySequence(QStringLiteral("Ctrl+L")), [this] {
            address_->setFocus();
            address_->selectAll();
        });
        shortcut(QKeySequence::Quit, [this] { close(); });

        statusBar()->showMessage(QStringLiteral("listo"));
        newTab(initial);
    }

protected:
    void closeEvent(QCloseEvent *event) override {
        while (tabs_->count() > 0) {
            auto *view = qobject_cast<QWebEngineView *>(tabs_->widget(0));
            tabs_->removeTab(0);
            delete view;
        }
        event->accept();
    }

private:
    QWebEngineView *view() const {
        return qobject_cast<QWebEngineView *>(tabs_->currentWidget());
    }

    void navigate(const QString &input) {
        QString text = input.trimmed();
        if (text.isEmpty()) {
            return;
        }
        QUrl url = QUrl::fromUserInput(text);
        view()->setUrl(url);
        address_->setText(url.toString());
    }

    void newTab(const QUrl &url) {
        auto *view = new QWebEngineView(this);
        const int index = tabs_->addTab(view, QStringLiteral("Nueva pestaña"));
        tabs_->setCurrentIndex(index);
        connect(view, &QWebEngineView::urlChanged, this, [this, view](const QUrl &url) {
            if (view == this->view()) {
                address_->setText(url.toString());
            }
        });
        connect(view, &QWebEngineView::titleChanged, this, [this, view](const QString &title) {
            const int index = tabs_->indexOf(view);
            if (index >= 0) {
                tabs_->setTabText(index, title.isEmpty() ? QStringLiteral("Nueva pestaña")
                                                         : title.left(24));
            }
            if (view == this->view()) {
                setWindowTitle(QStringLiteral("%1 — Navegador").arg(title));
            }
        });
        connect(view, &QWebEngineView::loadStarted, this, [this] {
            statusBar()->showMessage(QStringLiteral("cargando…"));
        });
        connect(view, &QWebEngineView::loadFinished, this, [this](bool ok) {
            statusBar()->showMessage(ok ? QStringLiteral("listo")
                                        : QStringLiteral("no se pudo cargar la página"));
        });
        connect(view, &QWebEngineView::loadProgress, this, [this](int progress) {
            if (progress < 100) {
                statusBar()->showMessage(QStringLiteral("cargando… %1 %").arg(progress));
            }
        });
        connect(view->page(), &QWebEnginePage::linkHovered, this,
                [this](const QString &link) { statusBar()->showMessage(link); });
        if (url.isValid() && !url.isEmpty()) {
            view->setUrl(url);
        } else {
            view->setUrl(QUrl(QStringLiteral("about:blank")));
        }
        view->setFocus();
    }

    void closeTab(int index) {
        if (index < 0) {
            return;
        }
        auto *view = qobject_cast<QWebEngineView *>(tabs_->widget(index));
        tabs_->removeTab(index);
        delete view;
        if (tabs_->count() == 0) {
            newTab(QUrl());
        }
    }

    void syncAddress() {
        if (auto *view = this->view()) {
            address_->setText(view->url().toString());
        }
    }

    QLineEdit *address_ = nullptr;
    QTabWidget *tabs_ = nullptr;
};

int main(int argc, char **argv) {
    // El agente se engancha por CDP al Chromium de Qt WebEngine.
    if (qEnvironmentVariableIsEmpty("QTWEBENGINE_REMOTE_DEBUGGING")) {
        qputenv("QTWEBENGINE_REMOTE_DEBUGGING", "127.0.0.1:9333");
    }

    QApplication app(argc, argv);
    app.setApplicationName(QStringLiteral("Navegador"));
    app.setDesktopFileName(QStringLiteral("navegador"));

    const QString data = QStandardPaths::writableLocation(QStandardPaths::AppDataLocation);
    QDir().mkpath(data);
    auto *profile = QWebEngineProfile::defaultProfile();
    profile->setPersistentStoragePath(data + QStringLiteral("/perfil"));
    profile->setCachePath(data + QStringLiteral("/cache"));
    profile->setPersistentCookiesPolicy(QWebEngineProfile::ForcePersistentCookies);
    profile->setHttpCacheType(QWebEngineProfile::DiskHttpCache);
    const QString downloads = QStandardPaths::writableLocation(QStandardPaths::DownloadLocation);
    QDir().mkpath(downloads);
    profile->setDownloadPath(downloads);

    QObject::connect(profile, &QWebEngineProfile::downloadRequested, &app,
                     [&app](QWebEngineDownloadRequest *download) {
                         download->setDownloadDirectory(
                             QStandardPaths::writableLocation(QStandardPaths::DownloadLocation));
                         download->accept();
                         QObject::connect(download,
                                          &QWebEngineDownloadRequest::stateChanged,
                                          &app, [download](QWebEngineDownloadRequest::DownloadState state) {
                                              if (state == QWebEngineDownloadRequest::DownloadCompleted) {
                                                  qInfo("descarga completa: %s",
                                                        qUtf8Printable(download->downloadFileName()));
                                              }
                                          });
                     });

    const QStringList args = app.arguments();
    QUrl initial;
    for (int i = 1; i < args.size(); ++i) {
        if (!args[i].startsWith(QLatin1Char('-'))) {
            initial = QUrl::fromUserInput(args[i]);
            break;
        }
    }
    if (initial.isEmpty()) {
        initial = QUrl(QStringLiteral("about:blank"));
    }

    Shell shell(initial);
    shell.show();
    return app.exec();
}
