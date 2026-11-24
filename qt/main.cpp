// Candidato experimental de escritorio con la vista nativa de Qt WebEngine.

#include <QAction>
#include <QApplication>
#include <QCloseEvent>
#include <QCommandLineParser>
#include <QFile>
#include <QFileDialog>
#include <QInputDialog>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QLockFile>
#include <QMessageBox>
#include <QMetaEnum>
#include <QMenu>
#include <QMenuBar>
#include <QProgressDialog>
#include <QRegularExpression>
#include <QSaveFile>
#include <QDir>
#include <QFileInfo>
#include <QKeySequence>
#include <QLineEdit>
#include <QMainWindow>
#include <QShortcut>
#include <QStandardPaths>
#include <QStatusBar>
#include <QTabBar>
#include <QTabWidget>
#include <QTimer>
#include <QToolBar>
#include <QSizePolicy>
#include <QUrl>
#include <QUrlQuery>
#include <QWebEngineCookieStore>
#include <QWebEngineDownloadRequest>
#include <QWebEngineNewWindowRequest>
#include <QWebEnginePermission>
#include <QWebEngineProfile>
#include <QWebEngineView>
#include <memory>
#include <utility>

#include "privacy.h"

class Shell : public QMainWindow {
public:
    Shell(QWebEngineProfile *profile, QString sessionPath, bool restore, const QList<QUrl> &initial)
        : profile_(profile), sessionPath_(std::move(sessionPath)),
          exceptions_(std::make_shared<StorageExceptions>(!profile->isOffTheRecord())) {
        setWindowTitle(QStringLiteral("Navegador Qt · experimental"));
        resize(1100, 780);

        profile_->cookieStore()->setCookieFilter([exceptions = exceptions_](
            const QWebEngineCookieStore::FilterRequest &request) {
            return !request.thirdParty || exceptions->allows(request.firstPartyUrl);
        });
        auto *privacy = menuBar()->addMenu(QStringLiteral("&Privacidad"));
        auto *storage = privacy->addAction(QStringLiteral("Permitir almacenamiento de terceros aquí"));
        storage->setCheckable(true);
        connect(privacy, &QMenu::aboutToShow, this, [this, storage] {
            const QUrl url = view() ? view()->url() : QUrl();
            storage->setEnabled(!StorageExceptions::origin(url).isEmpty());
            storage->setChecked(exceptions_->allows(url));
        });
        connect(storage, &QAction::triggered, this, [this](bool allowed) {
            if (auto *page = view()) {
                exceptions_->set(page->url(), allowed);
                page->reload();
            }
        });

        auto *toolbar = addToolBar(QStringLiteral("Navegación"));
        toolbar->setMovable(false);
        auto *back = toolbar->addAction(QStringLiteral("Atrás"));
        auto *forward = toolbar->addAction(QStringLiteral("Adelante"));
        auto *reload = toolbar->addAction(QStringLiteral("Recargar"));
        address_ = new QLineEdit(this);
        address_->setClearButtonEnabled(true);
        address_->setPlaceholderText(QStringLiteral("Busca o escribe una dirección"));
        address_->setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Preferred);
        address_->setMinimumWidth(120);
        toolbar->addWidget(address_);

        tabs_ = new QTabWidget(this);
        tabs_->setTabsClosable(true);
        tabs_->setMovable(true);
        setCentralWidget(tabs_);

        connect(back, &QAction::triggered, this, [this] { view()->back(); });
        connect(forward, &QAction::triggered, this, [this] { view()->forward(); });
        connect(reload, &QAction::triggered, this, [this] { view()->reload(); });
        connect(address_, &QLineEdit::returnPressed, this, [this] { navigate(address_->text()); });
        sessionTimer_.setSingleShot(true);
        sessionTimer_.setInterval(400);
        connect(&sessionTimer_, &QTimer::timeout, this, &Shell::saveSession);
        connect(tabs_, &QTabWidget::currentChanged, this, [this](int) {
            syncAddress();
            scheduleSessionSave();
        });
        connect(tabs_->tabBar(), &QTabBar::tabMoved, this, [this] { scheduleSessionSave(); });
        connect(tabs_, &QTabWidget::tabCloseRequested, this, &Shell::closeTab);

        auto shortcut = [this](const QKeySequence &key, auto action) {
            auto *shortcut = new QShortcut(key, this);
            connect(shortcut, &QShortcut::activated, this, action);
        };
        shortcut(QKeySequence(QStringLiteral("Ctrl+T")), [this] {
            newTab({});
            address_->clear();
            address_->setFocus();
        });
        shortcut(QKeySequence::Close, [this] { closeTab(tabs_->currentIndex()); });
        shortcut(QKeySequence::Refresh, [this] { view()->reload(); });
        shortcut(QKeySequence(QStringLiteral("Ctrl+L")), [this] {
            address_->setFocus();
            address_->selectAll();
        });
        shortcut(QKeySequence::Quit, [this] { close(); });
        shortcut(QKeySequence(QStringLiteral("Alt+Left")), [this] { view()->back(); });
        shortcut(QKeySequence(QStringLiteral("Alt+Right")), [this] { view()->forward(); });
        shortcut(QKeySequence(QStringLiteral("Ctrl++")), [this] {
            view()->setZoomFactor(qBound(0.5, view()->zoomFactor() + 0.1, 3.0));
        });
        shortcut(QKeySequence(QStringLiteral("Ctrl+-")), [this] {
            view()->setZoomFactor(qBound(0.5, view()->zoomFactor() - 0.1, 3.0));
        });
        shortcut(QKeySequence(QStringLiteral("Ctrl+0")), [this] { view()->setZoomFactor(1.0); });
        shortcut(QKeySequence::Find, [this] {
            bool accepted = false;
            const QString text = QInputDialog::getText(this, QStringLiteral("Buscar en la página"),
                QStringLiteral("Texto"), QLineEdit::Normal, {}, &accepted);
            if (accepted) view()->findText(text);
        });
        connect(profile_, &QWebEngineProfile::downloadRequested, this, &Shell::download);

        statusBar()->showMessage(QStringLiteral("listo"));
        if (restore && initial.isEmpty()) restoreSession();
        for (const QUrl &url : initial) newTab(url);
        if (tabs_->count() == 0) newTab({});
    }

protected:
    void closeEvent(QCloseEvent *event) override {
        closing_ = true;
        sessionTimer_.stop();
        saveSession();
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
        const QUrl url = addressUrl(text);
        view()->setUrl(url);
        address_->setText(url.toString());
    }

    static QUrl addressUrl(const QString &text) {
        if (!text.contains(QLatin1Char(' ')) &&
            (text.contains(QLatin1Char('.')) || text.contains(QLatin1Char(':')) ||
             text == QStringLiteral("localhost"))) {
            return QUrl::fromUserInput(text);
        }
        QUrl query(QStringLiteral("https://duckduckgo.com/"));
        QUrlQuery parameters;
        parameters.addQueryItem(QStringLiteral("q"), text);
        query.setQuery(parameters);
        return query;
    }

    QWebEngineView *newTab(const QUrl &url, bool activate = true) {
        auto *view = new QWebEngineView(profile_, this);
        const int index = tabs_->addTab(view, QStringLiteral("Nueva pestaña"));
        if (activate) tabs_->setCurrentIndex(index);
        connect(view, &QWebEngineView::urlChanged, this, [this, view](const QUrl &url) {
            if (view == this->view() && !address_->hasFocus()) {
                address_->setText(url.toString());
            }
            scheduleSessionSave();
        });
        connect(view, &QWebEngineView::titleChanged, this, [this, view](const QString &title) {
            const int index = tabs_->indexOf(view);
            if (index >= 0) {
                tabs_->setTabText(index, title.isEmpty() ? QStringLiteral("Nueva pestaña")
                                                         : title.left(24));
            }
            if (view == this->view()) {
                setWindowTitle(QStringLiteral("%1 — Navegador Qt · experimental").arg(title));
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
        connect(view->page(), &QWebEnginePage::newWindowRequested, this,
                [this](QWebEngineNewWindowRequest &request) {
                    if (!request.isUserInitiated()) return;
                    const bool activate = request.destination() !=
                                          QWebEngineNewWindowRequest::InNewBackgroundTab;
                    request.openIn(newTab({}, activate)->page());
                });
        connect(view->page(), &QWebEnginePage::permissionRequested, this,
                [this](QWebEnginePermission permission) {
                    const auto type = QMetaEnum::fromType<QWebEnginePermission::PermissionType>();
                    const auto *name = type.valueToKey(static_cast<int>(permission.permissionType()));
                    auto *dialog = new QMessageBox(QMessageBox::Question, QStringLiteral("Permiso del sitio"),
                        QStringLiteral("%1 solicita %2.")
                            .arg(permission.origin().toDisplayString(), QString::fromLatin1(name)),
                        QMessageBox::Yes | QMessageBox::No, this);
                    dialog->setDefaultButton(QMessageBox::No);
                    dialog->setTextFormat(Qt::PlainText);
                    dialog->setAttribute(Qt::WA_DeleteOnClose);
                    connect(dialog, &QMessageBox::finished, this, [permission](int result) {
                        if (!permission.isValid()) return;
                        if (result == QMessageBox::Yes) permission.grant();
                        else permission.deny();
                    });
                    dialog->open();
                });
        if (!url.isEmpty()) view->setUrl(url);
        if (activate) view->setFocus();
        return view;
    }

    void download(QWebEngineDownloadRequest *request) {
        const QString destination = QFileDialog::getSaveFileName(
            this, QStringLiteral("Guardar descarga"),
            QDir(request->downloadDirectory()).filePath(request->downloadFileName()),
            {}, nullptr, QFileDialog::DontUseNativeDialog);
        if (destination.isEmpty()) {
            request->cancel();
            return;
        }
        const QFileInfo file(destination);
        request->setDownloadDirectory(file.absolutePath());
        request->setDownloadFileName(file.fileName());
        auto *dialog = new QProgressDialog(file.fileName(), QStringLiteral("Cancelar"), 0, 0, this);
        dialog->setWindowTitle(QStringLiteral("Descarga"));
        dialog->setAutoClose(false);
        dialog->setAutoReset(false);
        dialog->setMinimumDuration(0);
        connect(dialog, &QProgressDialog::canceled, request, &QWebEngineDownloadRequest::cancel);
        connect(request, &QWebEngineDownloadRequest::receivedBytesChanged, dialog, [request, dialog] {
            const auto total = request->totalBytes();
            if (total > 0) {
                dialog->setRange(0, 100);
                dialog->setValue(qBound(0, int(100.0 * request->receivedBytes() / total), 100));
            }
        });
        connect(request, &QWebEngineDownloadRequest::isFinishedChanged, dialog, [this, request, dialog] {
            if (!request->isFinished()) return;
            if (request->state() == QWebEngineDownloadRequest::DownloadCompleted)
                statusBar()->showMessage(QStringLiteral("Descarga completa: %1").arg(request->downloadFileName()), 5000);
            else
                statusBar()->showMessage(QStringLiteral("Descarga finalizada: %1").arg(request->interruptReasonString()), 5000);
            dialog->deleteLater();
        });
        connect(request, &QObject::destroyed, dialog, &QObject::deleteLater);
        request->accept();
        if (!request->isFinished()) dialog->show();
    }

    static bool restorable(const QUrl &url) {
        return url.isValid() && !url.host().isEmpty() &&
               (url.scheme() == QStringLiteral("http") || url.scheme() == QStringLiteral("https"));
    }

    void scheduleSessionSave() {
        if (!closing_ && !sessionPath_.isEmpty()) sessionTimer_.start();
    }

    void saveSession() const {
        if (sessionPath_.isEmpty()) return;
        QJsonArray urls;
        int selected = 0;
        for (int i = 0; i < tabs_->count(); ++i) {
            const auto *tab = qobject_cast<QWebEngineView *>(tabs_->widget(i));
            const QUrl url = tab->url().isEmpty() ? tab->page()->requestedUrl() : tab->url();
            if (!restorable(url)) continue;
            if (i == tabs_->currentIndex()) selected = urls.size();
            urls.append(url.toString(QUrl::FullyEncoded));
        }
        QSaveFile file(sessionPath_);
        const auto data = QJsonDocument(QJsonObject{{"urls", urls}, {"active", selected}}).toJson();
        if (!file.open(QIODevice::WriteOnly) || file.write(data) != data.size() || !file.commit())
            qWarning("No se pudo guardar la sesión Qt");
    }

    void restoreSession() {
        QFile file(sessionPath_);
        if (!file.open(QIODevice::ReadOnly)) return;
        const auto saved = QJsonDocument::fromJson(file.readAll()).object();
        for (const auto &entry : saved.value("urls").toArray()) {
            const QUrl url(entry.toString(), QUrl::StrictMode);
            if (restorable(url)) newTab(url, false);
        }
        if (tabs_->count() > 0)
            tabs_->setCurrentIndex(qBound(0, saved.value("active").toInt(), tabs_->count() - 1));
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
        scheduleSessionSave();
    }

    void syncAddress() {
        if (auto *view = this->view()) {
            address_->setText(view->url().toString());
        }
    }

    QWebEngineProfile *profile_;
    QString sessionPath_;
    std::shared_ptr<StorageExceptions> exceptions_;
    QTimer sessionTimer_;
    bool closing_ = false;
    QLineEdit *address_ = nullptr;
    QTabWidget *tabs_ = nullptr;
};

int main(int argc, char **argv) {
    // CDP es optativo y solo admite un puerto en loopback para las pruebas del agente.
    const QByteArray debug = qgetenv("QTWEBENGINE_REMOTE_DEBUGGING");
    if (!QRegularExpression(QStringLiteral("^127[.]0[.]0[.]1:[0-9]{1,5}$"))
             .match(QString::fromLatin1(debug)).hasMatch())
        qunsetenv("QTWEBENGINE_REMOTE_DEBUGGING");

    QApplication app(argc, argv);
    app.setApplicationName(QStringLiteral("navegador-qt"));
    app.setApplicationVersion(QStringLiteral("0.1.0"));
    app.setDesktopFileName(QStringLiteral("navegador-qt"));

    QCommandLineParser parser;
    parser.setApplicationDescription(QStringLiteral("Vista nativa experimental de Navegador"));
    parser.addHelpOption();
    parser.addVersionOption();
    parser.addOption({"efimero", QStringLiteral("Usa un perfil temporal y no guarda la sesión.")});
    parser.addOption({"sin-restaurar", QStringLiteral("Abre una sesión nueva.")});
    parser.addPositionalArgument("url", QStringLiteral("Dirección inicial."));
    parser.process(app);

    std::unique_ptr<QLockFile> lock;
    std::unique_ptr<QWebEngineProfile> profile;
    QString sessionPath;
    if (parser.isSet("efimero")) {
        profile = std::make_unique<QWebEngineProfile>();
    } else {
        const QString data = QStandardPaths::writableLocation(QStandardPaths::AppDataLocation);
        if (!QDir().mkpath(data)) {
            qCritical("No se pudo crear el directorio del perfil Qt");
            return 1;
        }
        lock = std::make_unique<QLockFile>(QDir(data).filePath("perfil.lock"));
        lock->setStaleLockTime(0);
        if (!lock->tryLock()) {
            qCritical("El perfil Qt ya está en uso");
            return 1;
        }
        profile = std::make_unique<QWebEngineProfile>(QStringLiteral("principal"));
        profile->setPersistentStoragePath(QDir(data).filePath("perfil"));
        profile->setCachePath(QDir(QStandardPaths::writableLocation(QStandardPaths::CacheLocation)).filePath("web"));
        sessionPath = QDir(data).filePath("sesion.json");
    }
    const QString customDownloads = qEnvironmentVariable("NAVEGADOR_DESCARGAS");
    if (!customDownloads.isEmpty()) {
        if (!QDir().mkpath(customDownloads)) {
            qCritical("No se pudo crear el directorio de descargas");
            return 1;
        }
        profile->setDownloadPath(customDownloads);
    }
    QList<QUrl> initial;
    for (const auto &url : parser.positionalArguments()) initial.append(QUrl::fromUserInput(url));
    Shell shell(profile.get(), sessionPath, !parser.isSet("sin-restaurar"), initial);
    shell.show();
    return app.exec();
}
