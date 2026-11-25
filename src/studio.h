#pragma once
#include "circuit.h"
#include <QGraphicsView>
#include <QJsonArray>
#include <QMainWindow>
#include <QFileSystemModel>
#include <QTabWidget>
#include <QTreeView>
#include <QListWidget>
#include <QUndoStack>
#include <QToolBar>
#include <QProcess>
#include <QSet>
#include <functional>
inline constexpr auto partMime = "application/x-litzsim-component";
QFont studioFont();

class Sheet : public QGraphicsView {
public:
    Sheet(const QString& filename,Circuit value,QWidget* parent);
    ~Sheet() override { history.blockSignals(true); }
    QString path;
    Circuit circuit;
    QUndoStack history;
    void edit(const std::function<void(Circuit&)>& operation);
    virtual void redraw();
    static QJsonObject layout(const Circuit& circuit);
    void save();
    void exportSchematic(const QString& filename,double widthMm,bool annotations=false) const;
    void addPart(const QString& type,QPointF at);
protected:
    void mousePressEvent(QMouseEvent*) override;
    void mouseMoveEvent(QMouseEvent*) override;
    void mouseReleaseEvent(QMouseEvent*) override;
    void mouseDoubleClickEvent(QMouseEvent*) override;
    void keyPressEvent(QKeyEvent*) override;
    void dragEnterEvent(QDragEnterEvent*) override;
    void dragMoveEvent(QDragMoveEvent*) override;
    void dropEvent(QDropEvent*) override;
    void drawBackground(QPainter*,const QRectF&) override;
    void wheelEvent(QWheelEvent*) override;
private:
    struct WireSegment {
        QPoint a,b;
        QString pin;
    };
    struct NetRoute {
        QJsonArray pins,trunk;
        QMap<QString,QPoint> terminals;
        QJsonObject bends;
        QList<WireSegment> segments;
    };
    static void drawSchematic(QGraphicsScene* scene,const Circuit& circuit,QMap<QString,NetRoute>& routes,
                              QSet<QPoint>& routeCrossings,const QString& selected,const QString& pending,
                              const QString& activeNet);
    QMap<QString,NetRoute> routes;
    QSet<QPoint> routeCrossings;
    QString selected,pending,dragging,wirePin,activeNet;
    QPointF dragOffset;
};
class Studio : public QMainWindow {
public:
    explicit Studio(const QString& directory);
    Sheet* open(const QString& filename);
    Sheet* current() const;
    void refreshTabs();
    void startRun();
    void stopRun();
    QString runDirectory;
    std::function<void(const QString&)> showResult;
protected:
    void closeEvent(QCloseEvent* event) override;
private:
    void fileAction(const QString& action);
    bool confirmClose(Sheet* sheet);
    QString root;
    QFileSystemModel* files;
    QTreeView* tree;
    QListWidget* parts;
    QTabWidget* tabs;
    QToolBar* bar;
    QProcess* runner;
    QString runState;
    void recordRun(const QString& state,const QString& message={});
};
