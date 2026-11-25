#pragma once
#include "waves.h"
#include <QMainWindow>
#include <QStackedLayout>
#include <QTemporaryDir>
#include <QToolBar>
#include <QVBoxLayout>
#include <functional>
#include <memory>

struct Signal {
    std::shared_ptr<Waves> data;
    QString source;
    int x=0,y=1;
    bool right=false;
};
struct Scale {
    double low=0,high=1;
    bool automatic=true;
};
class Plot : public QWidget {
public:
    explicit Plot(QWidget* parent=nullptr);
    std::vector<Signal> signalsList;
    Scale x,left,right;
    std::function<void(Plot*)> selected,changed;
    void fit();
protected:
    void paintEvent(QPaintEvent*) override;
    void wheelEvent(QWheelEvent*) override;
    void mousePressEvent(QMouseEvent*) override;
    void mouseMoveEvent(QMouseEvent*) override;
    void mouseDoubleClickEvent(QMouseEvent*) override;
private:
    QPoint drag;
    QRect graph() const;
};
class Viewer : public QMainWindow {
public:
    explicit Viewer(QWidget* parent=nullptr);
    void open(const QString& path);
    Plot* addPlot();
    Plot* active=nullptr;
private:
    void addSource(const QString& path,const QString& label);
    void chooseSignal();
    void ranges();
    QVBoxLayout* plots;
    QAction* link;
    std::vector<std::pair<QString,std::shared_ptr<Waves>>> files;
    QTemporaryDir cache;
};
