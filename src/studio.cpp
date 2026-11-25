#include "studio.h"
#include <QMimeData>
#include "viewer.h"
#include <QPointer>
#include <QCloseEvent>
#include <QDir>
#include <QApplication>
#include <QJsonDocument>
#include <QSaveFile>
#include <QUuid>
#include <QDialogButtonBox>
#include <QFormLayout>
#include <QComboBox>
#include <QFontDatabase>
#include <QStyleHints>
#include <QLabel>
#include <QPainter>
#include <QPainterPath>
#include <QFileDialog>
#include <QFileInfo>
#include <QFileIconProvider>
#include <QGraphicsScene>
#include <QInputDialog>
#include <QMenu>
#include <QMessageBox>
#include <QSplitter>
#include <QStackedWidget>
#include <QStatusBar>
#include <QToolButton>
#include <QTimer>
#include <QVBoxLayout>

namespace {
class PartList : public QListWidget {
    QMimeData* mimeData(const QList<QListWidgetItem*>& items) const override {
        auto* data=new QMimeData;
        if(items.size()==1) data->setData(partMime,items.first()->text().toUtf8());
        return data;
    }
};
QIcon icon(const QString& name) {return QIcon(":/icons/"+name+".svg");}
class FileIcons : public QFileIconProvider {
public:
    QIcon icon(const QFileInfo& info) const override {return ::icon(info.isDir() ? "Folder":"File");}
};
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
    static const int fontId=QFontDatabase::addApplicationFont(":/fonts/PretendardVariable.ttf");
    if (fontId<0) throw std::runtime_error("cannot load bundled Pretendard font");
    QFont ui(QFontDatabase::applicationFontFamilies(fontId).front());
    ui.setPixelSize(14);
    QApplication::setFont(ui);
#if QT_VERSION >= QT_VERSION_CHECK(6, 8, 0)
    QGuiApplication::styleHints()->setColorScheme(Qt::ColorScheme::Light);
#endif
    resize(1220,780); setWindowTitle("LitzSim");
    QApplication::setWindowIcon(QIcon(":/icons/litzio.ico"));
    bar=addToolBar("회로");
    bar->setMovable(false);
    auto* mark=new QLabel;mark->setContentsMargins(12,0,0,0);
    mark->setPixmap(QApplication::windowIcon().pixmap(24,24));bar->addWidget(mark);
    auto* brand=new QLabel("LitzSim");brand->setObjectName("brand");bar->addWidget(brand);bar->addSeparator();
    auto action=[&](const QString& label,const QKeySequence& key,auto fn) {
        auto* a=bar->addAction(label); a->setShortcut(key);
        if(label=="열기") a->setIcon(icon("FolderOpen"));
        if(label=="해석 설정") a->setIcon(icon("Settings2"));
        if(label=="실행") a->setIcon(icon("Play"));
        if(label=="중지") a->setIcon(icon("Square"));
        bar->setToolButtonStyle(Qt::ToolButtonTextBesideIcon);
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
    runner=new QProcess(this);
    connect(runner,&QProcess::finished,this,[this](int code,QProcess::ExitStatus status){
        QString state=runState=="stopping" ? "canceled" : (status==QProcess::NormalExit && code==0 ? "completed" : "failed");
        QString detail=QString::fromUtf8(runner->readAllStandardError()).trimmed();
        recordRun(state,detail);
        if (state=="completed" && showResult) showResult(runDirectory+"/result.wave");
    });
    connect(runner,&QProcess::errorOccurred,this,[this](QProcess::ProcessError e){
        if (e==QProcess::FailedToStart) recordRun("failed",runner->errorString());
    });
    bar->addSeparator();
    action("해석 설정",{},[this]{
        auto* sheet=current();if (!sheet) return;
        auto cfg=sheet->circuit.document["simulation"].toObject();
        QDialog dialog(this);dialog.setWindowTitle("해석 설정");
        QFormLayout form(&dialog);
        QLineEdit step(QString::number(cfg["timeStep_s"].toDouble(),'g',16));
        QLineEdit stop(QString::number(cfg["stopTime_s"].toDouble(),'g',16));
        form.addRow("Time-step [s]",&step);form.addRow("종료 시각 [s]",&stop);
        QLineEdit output;
        output.setPlaceholderText("자동 · 모든 해석 표본");
        output.setToolTip("Time-step의 정수 배로 입력합니다.");
        if (cfg.contains("outputStep_s")) output.setText(QString::number(cfg["outputStep_s"].toDouble(),'g',16));
        form.addRow("결과 저장 간격 [s]",&output);
        QComboBox model;
        if(sheet->circuit.document.contains("control")) {
            model.addItems({"pfc-switching","pfc-average"});model.setCurrentText(cfg["model"].toString());
            form.addRow("전력단 모델",&model);
        }
        QDialogButtonBox buttons(QDialogButtonBox::Ok|QDialogButtonBox::Cancel);
        form.addRow(&buttons);
        connect(&buttons,&QDialogButtonBox::accepted,&dialog,&QDialog::accept);
        connect(&buttons,&QDialogButtonBox::rejected,&dialog,&QDialog::reject);
        if (dialog.exec()!=QDialog::Accepted) return;
        cfg["timeStep_s"]=step.text().toDouble();cfg["stopTime_s"]=stop.text().toDouble();
        if (output.text().trimmed().isEmpty()) cfg.remove("outputStep_s");
        else cfg["outputStep_s"]=output.text().toDouble();
        if(model.count()) cfg["model"]=model.currentText();
        sheet->edit([&](Circuit& c){c.document["simulation"]=cfg;});
    });
    action("실행",Qt::Key_F5,[this]{startRun();});
    action("중지",Qt::SHIFT|Qt::Key_F5,[this]{stopRun();});
    auto viewer=std::make_shared<QPointer<Viewer>>();
    showResult=[this,viewer](const QString& path){
        if (!*viewer) *viewer=new Viewer(this);
        if (!path.isEmpty()) (*viewer)->open(path);
        (*viewer)->show();(*viewer)->raise();
    };
    action("파형 창",{},[this]{showResult({});});

    auto* frame=new QWidget;
    auto* row=new QHBoxLayout(frame); row->setSpacing(0); row->setContentsMargins(0,0,0,0);
    auto* rail=new QWidget; auto* railLayout=new QVBoxLayout(rail);
    rail->setObjectName("activity");rail->setFixedWidth(72);
    railLayout->setContentsMargins(3,6,3,6);
    auto* panels=new QStackedWidget;
    auto* split=new QSplitter;
    files=new QFileSystemModel(this);static FileIcons fileIcons;
    files->setIconProvider(&fileIcons);files->setRootPath(root);
    files->setNameFilters({"*.litzsim"}); files->setNameFilterDisables(false);
    tree=new QTreeView; tree->setModel(files); tree->setRootIndex(files->index(root));
    tree->setHeaderHidden(true);
    for (int i=1;i<files->columnCount();++i) tree->hideColumn(i);
    connect(tree,&QTreeView::doubleClicked,this,[this](const QModelIndex& index){
        auto path=files->filePath(index);
        if (QFileInfo(path).isFile()) try { open(path); } catch(const std::exception& e){ QMessageBox::warning(this,"열기",e.what()); }
    });
    tree->setContextMenuPolicy(Qt::CustomContextMenu);
    for(const auto name:{"새 회로","새 폴더","이름 변경","이동","삭제"}) {
        const auto label=QString::fromUtf8(name);
        auto* action=new QAction(label,tree);
        tree->addAction(action);
        connect(action,&QAction::triggered,this,[this,label] {
            try { fileAction(label); } catch(const std::exception& e){ QMessageBox::warning(this,"탐색기",e.what()); }
        });
    }
    connect(tree,&QWidget::customContextMenuRequested,this,[this](QPoint point){
        tree->setCurrentIndex(tree->indexAt(point));
        QMenu menu;
        menu.addActions(tree->actions());
        menu.exec(tree->viewport()->mapToGlobal(point));
    });
    parts=new PartList;
    parts->addItems(library().keys()); parts->setDragEnabled(true);
    connect(parts,&QListWidget::itemClicked,this,[this](QListWidgetItem* item){
        if (auto* s=current()) s->addPart(item->text(),s->mapToScene(s->viewport()->rect().center()));
    });
    auto side=[&](QString heading,QWidget* content){
        auto* panel=new QWidget;panel->setObjectName("panel");auto* layout=new QVBoxLayout(panel);
        layout->setContentsMargins(0,0,0,0);layout->setSpacing(0);
        auto* title=new QLabel(heading);title->setObjectName("paneHeader");layout->addWidget(title);layout->addWidget(content);
        panels->addWidget(panel);
    };
    side("탐색기",tree);side("부품 라이브러리",parts);
    for (const auto label:{"탐색기","라이브러리"}) {
        int n=railLayout->count();
        auto* button=new QToolButton; button->setText(QString::fromUtf8(label)); button->setCheckable(true);
        button->setObjectName("railItem");button->setIcon(icon(n==1?"LayoutGrid":"Files"));button->setIconSize(QSize(20,20));
        button->setToolButtonStyle(Qt::ToolButtonTextUnderIcon);button->setFixedSize(66,52);
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
    panels->resize(240,600);
    split->addWidget(panels); split->addWidget(tabs); split->setStretchFactor(1,1); split->setSizes({240,900});
    row->addWidget(rail); row->addWidget(split,1); setCentralWidget(frame);
    setStyleSheet(R"(
        QWidget { font-family:'Pretendard Variable'; font-size:14px; color:#232633; }
        QToolBar { background:#f6f7f9; border:0; border-bottom:1px solid #d5dae4; spacing:4px; min-height:40px; }
        QLabel#brand { font-weight:600; padding:0 16px; }
        QToolButton { padding:5px 8px; border:0; border-radius:4px; }
        QToolButton:hover { background:#e8eaf0; }
        QToolButton:focus,QTreeView:focus,QListWidget:focus,QTabBar:focus {outline:1px solid #803782;}
        QWidget#activity,QWidget#panel,QTreeView,QListWidget {background:#f6f7f9;}
        QToolButton#railItem {font-size:12px; padding:2px 0; border-left:3px solid transparent;}
        QToolButton#railItem:checked {color:#77277f; background:#f3eaf4; border-left:3px solid #803782;}
        QLabel#paneHeader {font-weight:600; padding:7px 12px; border-bottom:1px solid #e0e3e9;}
        QTreeView,QListWidget {border:0; padding:5px;}
        QTreeView::item,QListWidget::item {min-height:25px;}
        QTreeView::item:selected,QListWidget::item:selected {color:#52295e;background:#eee4f1;}
        QTabWidget::pane,QGraphicsView {border:0;background:white;}
        QTabBar,QTabWidget {background:#f6f7f9;}
        QTabBar::tab {min-height:30px; padding:0 10px; border:0; border-bottom:3px solid transparent; background:#f4f5f7;}
        QTabBar::tab:selected {color:#492550; background:white; border-bottom:3px solid #883f91;}
        QTabBar::close-button {image:url(:/icons/X.svg);width:13px;height:13px;}
        QSplitter::handle {background:#d5dae4;width:1px;}
    )");
}
Sheet* Studio::open(const QString& path) {
    for (int i=0;i<tabs->count();++i) {
        auto* s=static_cast<Sheet*>(tabs->widget(i));
        if (s->path.compare(normalized(path),Qt::CaseInsensitive)==0) { tabs->setCurrentIndex(i); return s; }
    }
    auto* sheet=new Sheet(path,Circuit::load(path),tabs);
    tabs->setCurrentIndex(tabs->addTab(sheet,QFileInfo(path).fileName()));
    tree->setCurrentIndex(files->index(normalized(path)));
    QTimer::singleShot(0,sheet,[sheet]{
        auto bounds=sheet->scene()->itemsBoundingRect();
        if (!bounds.isEmpty()) sheet->fitInView(bounds.adjusted(-50,-50,50,50),Qt::KeepAspectRatio);
    });
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
    if (runner->state()!=QProcess::NotRunning) {stopRun();runner->waitForFinished(3000);}
    e->accept();
}
void Studio::recordRun(const QString& state,const QString& detail) {
    runState=state;
    QSaveFile file(runDirectory+"/status.json");
    const auto bytes=QJsonDocument(QJsonObject{{"state",state},{"message",detail},
        {"circuit","circuit.litzsim"},{"result","result.wave"}}).toJson();
    if (!file.open(QIODevice::WriteOnly) || file.write(bytes)!=bytes.size() || !file.commit()) {
        statusBar()->showMessage("실행 상태 기록 실패 · "+runDirectory);return;
    }
    statusBar()->showMessage(state+" · "+(detail.isEmpty() ? runDirectory : detail));
}
void Studio::startRun() {
    if (!current() || runner->state()!=QProcess::NotRunning) return;
    auto* sheet=current();
    runDirectory=QFileInfo(sheet->path).dir().filePath("runs/"+QUuid::createUuid().toString(QUuid::WithoutBraces));
    if (!QDir().mkpath(runDirectory)) throw std::runtime_error("cannot create run directory");
    sheet->circuit.save(runDirectory+"/circuit.litzsim");
    recordRun("running");
    runner->start(QCoreApplication::applicationDirPath()+"/litzsim-cli.exe",
        {"run",runDirectory+"/circuit.litzsim",runDirectory+"/result.wave"});
}
void Studio::stopRun() {
    if (runner->state()==QProcess::NotRunning) return;
    recordRun("stopping");runner->kill();
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
