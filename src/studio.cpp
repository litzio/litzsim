#include "studio.h"
#include <QCloseEvent>
#include <QDir>
#include <QFileDialog>
#include <QFileInfo>
#include <QGraphicsScene>
#include <QInputDialog>
#include <QMenu>
#include <QMessageBox>
#include <QSplitter>
#include <QStackedWidget>
#include <QStatusBar>
#include <QToolButton>
#include <QVBoxLayout>

namespace {
class Revision : public QUndoCommand {
public:
    Revision(Sheet& target,Circuit before,Circuit after):sheet(target),oldValue(before),newValue(after) {}
    void undo() override { sheet.circuit=oldValue; sheet.redraw(); }
    void redo() override { sheet.circuit=newValue; sheet.redraw(); }
private:
    Sheet& sheet;
    Circuit oldValue,newValue;
};
QString normalized(const QString& path) { return QDir::cleanPath(QFileInfo(path).absoluteFilePath()); }
bool within(const QString& file,const QString& dir) {
    return file.compare(dir,Qt::CaseInsensitive)==0 || file.startsWith(dir+"/",Qt::CaseInsensitive);
}
Circuit emptyCircuit() {
    return {QJsonObject{{"version",1},{"parts",QJsonObject{}},{"nets",QJsonObject{}},{"layout",QJsonObject{}},
        {"simulation",QJsonObject{{"stopTime_s",0.001},{"timeStep_s",1e-6}}}}};
}
}
Sheet::Sheet(const QString& name,Circuit value,QWidget* parent):
    QGraphicsView(parent),path(normalized(name)),circuit(value) {
    setScene(new QGraphicsScene(this));
    redraw();
}
void Sheet::edit(const std::function<void(Circuit&)>& operation) {
    Circuit next=circuit;
    operation(next); next.validate();
    if (next.document!=circuit.document) history.push(new Revision(*this,circuit,next));
}
void Sheet::save() { circuit.save(path); history.setClean(); }

Studio::Studio(const QString& directory):root(normalized(directory)) {
    resize(1220,780); setWindowTitle("LitzSim");
    bar=addToolBar("회로");
    bar->setMovable(false);
    auto action=[&](const QString& label,const QKeySequence& key,auto fn) {
        auto* a=bar->addAction(label); a->setShortcut(key);
        connect(a,&QAction::triggered,this,[this,fn]{
            try { fn(); } catch (const std::exception& e) { QMessageBox::warning(this,"LitzSim",e.what()); }
        });
    };
    action("새 회로",QKeySequence::New,[this]{fileAction("새 회로");});
    action("열기",QKeySequence::Open,[this]{
        QString p=QFileDialog::getOpenFileName(this,"회로 열기",root,"LitzSim (*.litzsim)");
        if (!p.isEmpty()) open(p);
    });
    action("저장",QKeySequence::Save,[this]{if (current()) current()->save();});
    action("실행 취소",QKeySequence::Undo,[this]{if (current()) current()->history.undo();});
    action("다시 실행",QKeySequence::Redo,[this]{if (current()) current()->history.redo();});

    auto* frame=new QWidget;
    auto* row=new QHBoxLayout(frame); row->setSpacing(0); row->setContentsMargins(0,0,0,0);
    auto* rail=new QWidget; auto* railLayout=new QVBoxLayout(rail);
    railLayout->setContentsMargins(3,6,3,6);
    auto* panels=new QStackedWidget;
    auto* split=new QSplitter;
    files=new QFileSystemModel(this); files->setRootPath(root);
    files->setNameFilters({"*.litzsim"}); files->setNameFilterDisables(false);
    tree=new QTreeView; tree->setModel(files); tree->setRootIndex(files->index(root));
    tree->setHeaderHidden(true);
    for (int i=1;i<files->columnCount();++i) tree->hideColumn(i);
    connect(tree,&QTreeView::doubleClicked,this,[this](const QModelIndex& index){
        auto path=files->filePath(index);
        if (QFileInfo(path).isFile()) try { open(path); } catch(const std::exception& e){ QMessageBox::warning(this,"열기",e.what()); }
    });
    tree->setContextMenuPolicy(Qt::CustomContextMenu);
    connect(tree,&QWidget::customContextMenuRequested,this,[this](QPoint point){
        tree->setCurrentIndex(tree->indexAt(point));
        QMenu menu;
        for (const auto name:{"새 회로","새 폴더","이름 변경","이동","삭제"}) menu.addAction(QString::fromUtf8(name));
        auto* selected=menu.exec(tree->viewport()->mapToGlobal(point));
        if (selected) try { fileAction(selected->text()); } catch(const std::exception& e){ QMessageBox::warning(this,"탐색기",e.what()); }
    });
    parts=new QListWidget;
    parts->addItems(library().keys()); parts->setDragEnabled(true);
    connect(parts,&QListWidget::itemClicked,this,[this](QListWidgetItem* item){
        if (auto* s=current()) s->addPart(item->text(),s->mapToScene(s->viewport()->rect().center()));
    });
    panels->addWidget(tree); panels->addWidget(parts);
    for (const auto label:{"탐색기","라이브러리"}) {
        int n=railLayout->count();
        auto* button=new QToolButton; button->setText(QString::fromUtf8(label)); button->setCheckable(true);
        button->setAutoExclusive(true); button->setChecked(n==0);
        connect(button,&QToolButton::clicked,panels,[panels,n]{panels->setCurrentIndex(n);});
        railLayout->addWidget(button);
    }
    railLayout->addStretch();
    tabs=new QTabWidget; tabs->setDocumentMode(true); tabs->setTabsClosable(true); tabs->setMovable(true);
    connect(tabs,&QTabWidget::tabCloseRequested,this,[this](int i){
        auto* sheet=static_cast<Sheet*>(tabs->widget(i));
        if (confirmClose(sheet)) { tabs->removeTab(i); delete sheet; }
    });
    split->addWidget(panels); split->addWidget(tabs); split->setStretchFactor(1,1); split->setSizes({225,900});
    row->addWidget(rail); row->addWidget(split,1); setCentralWidget(frame);
    setStyleSheet("QMainWindow,QToolBar{background:#eceef2} QToolButton:checked,QTabBar::tab:selected{background:#ede7f2;color:#563c67} QTreeView,QListWidget,QGraphicsView{background:white} QToolBar{spacing:6px}");
}
Sheet* Studio::open(const QString& path) {
    for (int i=0;i<tabs->count();++i) {
        auto* s=static_cast<Sheet*>(tabs->widget(i));
        if (s->path.compare(normalized(path),Qt::CaseInsensitive)==0) { tabs->setCurrentIndex(i); return s; }
    }
    auto* sheet=new Sheet(path,Circuit::load(path),tabs);
    tabs->setCurrentIndex(tabs->addTab(sheet,QFileInfo(path).fileName()));
    connect(&sheet->history,&QUndoStack::cleanChanged,this,[this]{refreshTabs();});
    return sheet;
}
Sheet* Studio::current() const { return static_cast<Sheet*>(tabs->currentWidget()); }
void Studio::refreshTabs() {
    for (int i=0;i<tabs->count();++i) {
        auto* sheet=static_cast<Sheet*>(tabs->widget(i));
        tabs->setTabText(i,QFileInfo(sheet->path).fileName()+(sheet->history.isClean() ? "" : " *"));
        tabs->setTabToolTip(i,sheet->path);
    }
}
bool Studio::confirmClose(Sheet* sheet) {
    if (sheet->history.isClean()) return true;
    auto answer=QMessageBox::question(this,"저장",QFileInfo(sheet->path).fileName()+" 변경 사항을 저장할까요?",
        QMessageBox::Save|QMessageBox::Discard|QMessageBox::Cancel,QMessageBox::Save);
    if (answer==QMessageBox::Save) try { sheet->save(); } catch(const std::exception& e) {
        QMessageBox::warning(this,"저장",e.what()); return false;
    }
    return answer!=QMessageBox::Cancel;
}
void Studio::closeEvent(QCloseEvent* e) {
    for (int i=0;i<tabs->count();++i) if (!confirmClose(static_cast<Sheet*>(tabs->widget(i)))) { e->ignore(); return; }
    e->accept();
}
void Studio::fileAction(const QString& op) {
    const auto index=tree->currentIndex();
    QString selected=index.isValid() ? normalized(files->filePath(index)) : root;
    QString dir=QFileInfo(selected).isDir() ? selected : QFileInfo(selected).absolutePath();
    if (op=="새 회로" || op=="새 폴더") {
        QString name=QInputDialog::getText(this,op,"이름").trimmed();
        if (name.isEmpty()) return;
        if (name=="." || name==".." || name.contains('/') || name.contains('\\') || name.contains(':')) throw std::runtime_error("invalid name");
        if (op=="새 회로" && !name.endsWith(".litzsim",Qt::CaseInsensitive)) name+=".litzsim";
        QString path=QDir(dir).filePath(name);
        if (QFileInfo::exists(path)) throw std::runtime_error("destination already exists");
        if (op=="새 폴더") { if (!QDir(dir).mkdir(name)) throw std::runtime_error("mkdir failed"); }
        else { emptyCircuit().save(path); open(path); }
        return;
    }
    if (selected==root || !within(selected,root)) return;
    if (op=="삭제") {
        for (int i=0;i<tabs->count();++i)
            if (within(static_cast<Sheet*>(tabs->widget(i))->path,selected)) throw std::runtime_error("close open files before deleting");
        if (QMessageBox::question(this,op,selected+" 항목을 휴지통으로 이동할까요?")!=QMessageBox::Yes) return;
        if (!QFile::moveToTrash(selected)) throw std::runtime_error("trash operation failed");
        return;
    }
    QString target;
    if (op=="이동") {
        QString folder=QFileDialog::getExistingDirectory(this,op,root);
        if (folder.isEmpty()) return;
        target=QDir(folder).filePath(QFileInfo(selected).fileName());
    } else {
        QString name=QInputDialog::getText(this,op,"새 이름",QLineEdit::Normal,QFileInfo(selected).fileName()).trimmed();
        if (name.isEmpty()) return;
        if (name=="." || name==".." || name.contains('/') || name.contains('\\') || name.contains(':')) throw std::runtime_error("invalid name");
        if (QFileInfo(selected).isFile() && !name.endsWith(".litzsim",Qt::CaseInsensitive)) name+=".litzsim";
        target=QFileInfo(selected).dir().filePath(name);
    }
    if (!QDir().rename(selected,target)) throw std::runtime_error("rename or move failed");
    for (int i=0;i<tabs->count();++i) {
        auto* sheet=static_cast<Sheet*>(tabs->widget(i));
        if (within(sheet->path,selected)) sheet->path=target+sheet->path.mid(selected.size());
    }
    refreshTabs();
}
