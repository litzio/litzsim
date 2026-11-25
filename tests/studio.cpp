#include "studio.h"
#include <QApplication>
#include <QTemporaryDir>
#include <QGraphicsSimpleTextItem>
int main(int argc,char** argv) {
    QApplication app(argc,argv);
    if (argc!=2) return 2;
    QTemporaryDir dir;
    Studio studio(dir.path());
    QString examples=QString::fromLocal8Bit(argv[1]);
    QString a=dir.filePath("a.litzsim"),b=dir.filePath("b.litzsim");
    Circuit::load(examples+"/rc.litzsim").save(a);
    Circuit::load(examples+"/buck.litzsim").save(b);
    auto* first=studio.open(a);
    first->edit([](Circuit& c){c.move("V1",12,3);});
    auto* second=studio.open(b);
    for (auto* item:second->scene()->items()) {
        auto* label=qgraphicsitem_cast<QGraphicsSimpleTextItem*>(item);
        if (label && label->text().startsWith("배선 경로 확인")) { qWarning()<<label->text();return 2; }
    }
    auto copy=second->circuit;
    copy.disconnectPin("C1.p");
    if (!copy.missingPins().contains("C1.p")) return 3;
    copy.connectPins("VC.p","C1.p");
    if (!copy.missingPins().isEmpty()) return 4;
    if (!second->history.isClean() || first->history.isClean() || second==first) return 1;
    if (studio.open(a)!=first || studio.current()!=first) return 1;
    first->save();
    if (Circuit::load(a).document!=first->circuit.document) return 1;
    first->history.undo();
    return first->history.isClean() ? 1 : 0;
}
