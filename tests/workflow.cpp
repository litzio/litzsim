#include "studio.h"
#include "viewer.h"
#include "solver.h"
#include <QApplication>
#include <QAction>
#include <QCheckBox>
#include <QComboBox>
#include <QDropEvent>
#include <QFileDialog>
#include <QFileSystemModel>
#include <QGraphicsSimpleTextItem>
#include <QInputDialog>
#include <QJsonArray>
#include <QLineEdit>
#include <QMimeData>
#include <QTemporaryDir>
#include <QToolButton>
#include <QTimer>
#include <QtTest>
#include <cmath>
#include <memory>
#include <stdexcept>

namespace {
void expect(bool condition,const char* message) {
    if(!condition) throw std::runtime_error(message);
}
QAction* action(const QList<QAction*>& actions,const QString& text) {
    for(auto* a:actions) if(a->text()==text) return a;
    throw std::runtime_error(("action missing: "+text).toStdString());
}
void input(const QString& text) {
    QTimer::singleShot(0,[text]{
        auto* d=qobject_cast<QInputDialog*>(QApplication::activeModalWidget());
        if(d){d->setTextValue(text);d->accept();}
    });
}
QPoint itemPoint(Sheet& sheet,QString key,bool pin=false) {
    for(auto* item:sheet.scene()->items()) {
        if(item->data(pin?1:0).toString()!=key) continue;
        if(!pin && !qgraphicsitem_cast<QGraphicsSimpleTextItem*>(item)) continue;
        auto p=item->sceneBoundingRect().center();
        sheet.ensureVisible(item,10,10);
        return sheet.mapFromScene(p);
    }
    throw std::runtime_error(("missing graphic: "+key).toStdString());
}
void moveTo(Sheet& sheet,const QString& id,QPoint destination) {
    const auto src=itemPoint(sheet,id);
    const auto old=sheet.circuit.document["layout"].toObject()[id].toObject();
    QPoint origin(old["x"].toInt(),old["y"].toInt());
    QPointF end=sheet.mapToScene(src)+(destination-origin)*32;
    QTest::mousePress(sheet.viewport(),Qt::LeftButton,{},src);
    QTest::mouseMove(sheet.viewport(),sheet.mapFromScene(end));
    QTest::mouseRelease(sheet.viewport(),Qt::LeftButton,{},sheet.mapFromScene(end));
}
QSet<QPair<QPoint,QPoint>> segments(const Sheet& sheet,const QString& net) {
    QSet<QPair<QPoint,QPoint>> result;
    for(auto* item:sheet.scene()->items()) {
        const auto* wire=qgraphicsitem_cast<QGraphicsLineItem*>(item);
        if(wire && wire->data(3).toString()==net)
            result.insert({wire->line().p1().toPoint(),wire->line().p2().toPoint()});
    }
    return result;
}
void crossingAndJunction() {
    Circuit circuit{QJsonObject{{"version",1},{"parts",QJsonObject{}},{"nets",QJsonObject{}},
        {"layout",QJsonObject{}},{"simulation",QJsonObject{{"stopTime_s",0.001},{"timeStep_s",1e-6}}}}};
    const auto left=circuit.insert("resistor",-4,0)+".n";
    const auto right=circuit.insert("resistor",4,0)+".p";
    const auto branch=circuit.insert("ground",1,3)+".ref";
    const auto top=circuit.insert("dc",0,-4)+".n";
    const auto bottom=circuit.insert("dc",0,4)+".p";
    circuit.connectPins(left,right);
    circuit.connectPins(left,branch);
    circuit.connectPins(top,bottom);
    QString horizontal,vertical;
    const auto nets=circuit.document["nets"].toObject();
    for(auto it=nets.begin();it!=nets.end();++it) {
        if(it.value().toArray().contains(left)) horizontal=it.key();
        if(it.value().toArray().contains(top)) vertical=it.key();
    }
    circuit.document["trunks"]=QJsonObject{
        {horizontal,QJsonArray{QJsonArray{-3,0},QJsonArray{3,0}}},
        {vertical,QJsonArray{QJsonArray{0,-3},QJsonArray{0,3}}}
    };
    Sheet sheet("",circuit,nullptr);
    int jumps=0,dots=0;
    QPainterPath arcInk;
    for(auto* item:sheet.scene()->items()) {
        if(const auto* arc=qgraphicsitem_cast<QGraphicsPathItem*>(item);arc && item->data(3)==horizontal) {
            const auto rect=arc->path().boundingRect();
            expect(rect.center().x()==0 && rect.bottom()==0 && rect.top()<0 &&
                   rect.width()==2*rect.height(),"horizontal semicircle at crossing");
            arcInk=arc->shape();
            ++jumps;
        }
        if(qgraphicsitem_cast<QGraphicsEllipseItem*>(item)) {
            expect(item->pos()==QPointF(32,0),"junction dot only at connected T");
            ++dots;
        }
    }
    expect(jumps==1 && dots==1,"one jump and one connected T");
    for(auto* item:sheet.scene()->items())
        if(qgraphicsitem_cast<QGraphicsLineItem*>(item)) {
            if(item->data(3)==vertical) expect(!arcInk.intersects(item->shape()),"vertical stroke must clear the arc");
            if(item->data(3)==horizontal) expect(!item->shape().contains({0,0}),"arc must replace straight stroke");
        }
    expect(sheet.circuit.document==circuit.document,"rendering changed connectivity");
}
void localRouting(const QString& examples) {
    auto circuit=Circuit::load(examples+"/rc.litzsim");
    Sheet sheet("",circuit,nullptr);
    auto before=segments(sheet,"input");
    expect(!before.isEmpty(),"initial input route");
    QString obstruction;
    // Pick a midpoint between the two pins to force a local detour.
    for(auto segment:before) {
        auto p=segment.first/32;
        if(p.x()==1) {
            sheet.edit([&](Circuit& c){obstruction=c.insert("dc",p.x(),p.y());});
            break;
        }
    }
    expect(!obstruction.isEmpty(),"place routing obstruction");
    auto detour=segments(sheet,"input");
    expect(!detour.isEmpty() && detour!=before,"routing around moved obstacle");
    sheet.edit([&](Circuit& c){c.move(obstruction,20,20);});
    expect(segments(sheet,"input")==detour,"unaffected wiring was rearranged");
}
void workbench(const QString& examples,const QString& root) {
    Studio studio(root);studio.resize(1400,850);studio.show();QApplication::processEvents();
    auto* tree=studio.findChild<QTreeView*>();
    auto* model=qobject_cast<QFileSystemModel*>(tree->model());
    input("work");action(tree->actions(),"새 폴더")->trigger();
    const auto folder=root+"/work";
    expect(QDir(folder).exists(),"create folder");
    tree->setCurrentIndex(model->index(folder));
    input("test-buck");action(tree->actions(),"새 회로")->trigger();
    auto* sheet=studio.current();
    expect(sheet && QFileInfo::exists(folder+"/test-buck.litzsim"),"create sheet");
    studio.open(examples+"/rc.litzsim");
    studio.open(sheet->path);
    for(auto* b:studio.findChildren<QToolButton*>()) if(b->text()=="라이브러리") b->click();
    auto* library=studio.findChild<QListWidget*>();
    const auto fixture=Circuit::load(examples+"/buck.litzsim");
    const auto parts=fixture.document["parts"].toObject(),layout=fixture.document["layout"].toObject();
    QMap<QString,QString> ids;
    for(auto it=parts.begin();it!=parts.end();++it) {
        const auto type=it.value().toObject()["type"].toString();
        auto* entry=library->findItems(type,Qt::MatchExactly).value(0);
        expect(entry,"library entry");
        const auto pos=layout[it.key()].toObject();
        QPoint dest(pos["x"].toInt(),pos["y"].toInt());
        auto before=sheet->circuit.document["parts"].toObject();
        if(ids.empty()) {
            QTest::mouseClick(library->viewport(),Qt::LeftButton,{},library->visualItemRect(entry).center());
        } else {
            auto row=library->model()->index(library->row(entry),0);
            std::unique_ptr<QMimeData> mime(library->model()->mimeData({row}));
            QPoint pixel=sheet->mapFromScene(dest*32);
            QDragEnterEvent enter(pixel,Qt::CopyAction,mime.get(),Qt::LeftButton,{});
            QApplication::sendEvent(sheet->viewport(),&enter);
            QDropEvent drop(pixel,Qt::CopyAction,mime.get(),Qt::LeftButton,{});
            QApplication::sendEvent(sheet->viewport(),&drop);
            expect(drop.isAccepted(),"drop rejected");
        }
        auto after=sheet->circuit.document["parts"].toObject();
        expect(after.size()==before.size()+1,"part addition");
        for(auto name:after.keys()) if(!before.contains(name)) ids[it.key()]=name;
        if(ids.size()==1) moveTo(*sheet,ids[it.key()],dest);
    }
    auto target=ids["R1"];
    const auto initial=sheet->circuit.document;
    const auto capacitor=layout["C1"].toObject();
    moveTo(*sheet,target,{capacitor["x"].toInt(),capacitor["y"].toInt()});
    expect(sheet->circuit.document==initial,"overlap changed circuit or layout");
    auto point=itemPoint(*sheet,ids["V1"]);
    QTest::mouseClick(sheet->viewport(),Qt::LeftButton,{},point);
    QTest::keyClick(sheet,Qt::Key_R);
    expect(sheet->circuit.document["layout"]!=initial["layout"],"rotate did not act");
    sheet->history.undo();expect(sheet->circuit.document==initial,"rotation undo");
    auto nets=fixture.document["nets"].toObject();
    for(auto net:nets) {
        QStringList pins;
        for(auto ref:net.toArray()){
            auto pin=ref.toString(); pins<<ids[pin.section('.',0,0)]+"."+pin.section('.',1);
        }
        for(int i=1;i<pins.size();++i) {
            for(auto pin:{pins[0],pins[i]})
                QTest::mouseClick(sheet->viewport(),Qt::LeftButton,{},itemPoint(*sheet,pin,true));
        }
    }
    expect(sheet->circuit.missingPins().empty(),"incomplete buck wiring");
    tree->setCurrentIndex(model->index(sheet->path));
    input("renamed");action(tree->actions(),"이름 변경")->trigger();
    expect(sheet->path==folder+"/renamed.litzsim","open sheet rename");
    tree->setCurrentIndex(model->index(sheet->path));
    QTimer::singleShot(50,[root]{
        auto* d=qobject_cast<QFileDialog*>(QApplication::activeModalWidget());
        if(d){
            d->setDirectory(root);
            QTimer::singleShot(0,d,[d]{static_cast<QDialog*>(d)->accept();});
        }
    });
    action(tree->actions(),"이동")->trigger();
    expect(sheet->path==root+"/renamed.litzsim","open sheet move");
    QTimer::singleShot(0,[]{
        auto* d=qobject_cast<QDialog*>(QApplication::activeModalWidget());
        if(!d)return;
        d->findChildren<QLineEdit*>().at(2)->setText("0.000007");
        d->accept();
    });
    action(studio.findChild<QToolBar*>()->actions(),"해석 설정")->trigger();
    expect(sheet->circuit.outputStride()==7,"edit recording interval");
    sheet->save();
    auto saved=Circuit::load(sheet->path);
    auto reference=fixture;
    reference.document["simulation"]=saved.document["simulation"];
    QList<double> voltages;
    simulate(reference,[&](double,const auto& values){voltages<<values["VC_V"];});
    qsizetype row=0;
    simulate(saved,[&](double,const auto& values){
        expect(std::abs(values[ids["VC"]+"_V"]-voltages.at(row++))<1e-8,"assembled buck voltage mismatch");
    });
    expect(row==voltages.size(),"buck sample count");
}
void plots(const QString& root) {
    Viewer viewer;viewer.show();
    auto path=root+"/loop.csv";QFile csv(path);
    expect(csv.open(QIODevice::WriteOnly),"create XY fixture");
    csv.write("time_s,x,y\n0,1,0\n1,0,1\n2,-1,0\n3,0,-1\n4,1,0\n");csv.close();
    viewer.open(path);
    expect(QTest::qWaitFor([&]{return !viewer.active->signalsList.empty();},5000),"CSV async open");
    auto* bar=viewer.findChild<QToolBar*>();auto* first=viewer.active;
    action(bar->actions(),"Plot 추가")->trigger();
    QTimer::singleShot(0,[]{
        auto* d=qobject_cast<QDialog*>(QApplication::activeModalWidget());
        if(!d) return;
        auto fields=d->findChildren<QComboBox*>();
        fields.at(1)->setCurrentText("x");fields.at(2)->setCurrentText("y");
        d->findChild<QCheckBox*>()->setChecked(true);d->accept();
    });
    action(bar->actions(),"신호 겹치기")->trigger();
    auto* xy=viewer.active;auto s=xy->signalsList.at(0);
    expect(s.right && s.x==1 && s.y==2 && first!=xy,"XY/right axis assignment");
    auto rows=s.data->envelopeRows(0,4,1,400);
    expect(rows==std::vector<qint64>({0,1,2,3,4}),"XY sequence order");
    QTimer::singleShot(0,[]{
        auto* d=qobject_cast<QDialog*>(QApplication::activeModalWidget());
        if(!d)return;
        d->findChildren<QCheckBox*>().first()->setChecked(false);
        auto limits=d->findChildren<QLineEdit*>();
        limits.at(0)->setText("-2");limits.at(1)->setText("2");d->accept();
    });
    action(bar->actions(),"축 설정")->trigger();
    expect(!xy->x.automatic && xy->x.low==-2 && xy->x.high==2,"manual x bounds");
    const auto center=xy->rect().center();
    QWheelEvent zoom(center,xy->mapToGlobal(center),{},QPoint(0,120),{}, {},Qt::NoScrollPhase,false);
    QApplication::sendEvent(xy,&zoom);
    expect(xy->x.high-xy->x.low<4,"scroll zoom");
    auto before=xy->x.low;
    QTest::mousePress(xy,Qt::LeftButton,{},center);QTest::mouseMove(xy,center+QPoint(25,0));
    QTest::mouseRelease(xy,Qt::LeftButton,{},center+QPoint(25,0));
    expect(before!=xy->x.low,"plot pan");
    action(bar->actions(),"전체 보기")->trigger();
    expect(xy->x.automatic,"fit axes");
    action(bar->actions(),"Plot 삭제")->trigger();
    expect(viewer.active==first,"remove plot");
}
}
int main(int argc,char** argv){
    QApplication::setAttribute(Qt::AA_DontUseNativeDialogs);
    QApplication app(argc,argv);
    if(argc!=2)return 2;
    QTemporaryDir dir;
    try {
        const auto examples=QString::fromLocal8Bit(argv[1]);
        crossingAndJunction();workbench(examples,dir.path());localRouting(examples);plots(dir.path());
    }
    catch(const std::exception& e){fprintf(stderr,"%s\n",e.what());return 1;}
}
