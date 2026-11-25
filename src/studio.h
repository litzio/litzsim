#pragma once
#include "circuit.h"
#include <QGraphicsView>
#include <QMainWindow>
#include <QFileSystemModel>
#include <QTabWidget>
#include <QTreeView>
#include <QListWidget>
#include <QUndoStack>
#include <QToolBar>
#include <functional>

class Sheet : public QGraphicsView {
public:
    Sheet(const QString& filename,Circuit value,QWidget* parent);
    QString path;
    Circuit circuit;
    QUndoStack history;
    void edit(const std::function<void(Circuit&)>& operation);
    virtual void redraw();
    void save();
};
class Studio : public QMainWindow {
public:
    explicit Studio(const QString& directory);
    Sheet* open(const QString& filename);
    Sheet* current() const;
    void refreshTabs();
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
};
