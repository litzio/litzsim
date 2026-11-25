#include "studio.h"
#include <QApplication>
#include <QDragEnterEvent>
#include <QDropEvent>
#include <QGraphicsItem>
#include <QGraphicsScene>
#include <QInputDialog>
#include <QJsonArray>
#include <QKeyEvent>
#include <QMimeData>
#include <QMouseEvent>
#include <QPainter>
#include <QQueue>
#include <QSet>
#include <QWheelEvent>
#include <cmath>

namespace {
constexpr int pitch=32;
QPoint grid(QPointF p) {return {qRound(p.x()/pitch),qRound(p.y()/pitch)};}
QPoint position(QJsonObject pos) {return {pos["x"].toInt(),pos["y"].toInt()};}
QPoint rotatePoint(QPoint p,int degrees) {
    for (int i=0;i<degrees;i+=90) p={-p.y(),p.x()};
    return p;
}
QList<QPoint> offsets(const QString& type) {
    if (type=="ground") return {{0,-1}};
    if (type=="nmos") return {{0,-1},{-1,0},{0,1}};
    if (type=="resistor" || type=="inductor") return {{-1,0},{1,0}};
    if (type=="diode") return {{0,1},{0,-1}};
    return {{0,-1},{0,1}};
}
QPainterPath glyph(const QString& t) {
    QPainterPath p;
    auto line=[&](double x,double y,double a,double b){p.moveTo(x,y);p.lineTo(a,b);};
    if (t=="resistor") {
        p.moveTo(-32,0);p.lineTo(-16,0);
        for (int k=0;k<6;++k) p.lineTo(-13+5*k,k%2 ? 5 : -5);
        p.lineTo(16,0);p.lineTo(32,0);
    } else if (t=="inductor") {
        p.moveTo(-32,0);p.lineTo(-16,0);
        for (int k=0;k<4;++k) p.cubicTo(-16+8*k,-12,-8+8*k,-12,-8+8*k,0);
        p.lineTo(32,0);
    } else if (t=="capacitor") {
        line(0,-32,0,-3);line(-12,-3,12,-3);line(-12,3,12,3);line(0,3,0,32);
    } else if (t=="ground") {
        line(0,-32,0,-4);line(-12,-4,12,-4);line(-8,1,8,1);line(-3,6,3,6);
    } else if (t=="diode") {
        line(0,32,0,7);p.moveTo(-8,7);p.lineTo(0,-7);p.lineTo(8,7);p.closeSubpath();
        line(-9,-7,9,-7);line(0,-7,0,-32);
    } else if (t=="nmos") {
        line(0,-32,0,-12);line(0,-12,-5,-12);line(-5,-14,-5,-8);
        line(-5,-3,-5,3);line(-5,8,-5,14);line(-5,12,0,12);line(0,12,0,32);
        line(-32,0,-10,0);line(-10,-12,-10,12);
        line(0,-16,11,-16);line(11,-16,11,-3);line(7,-3,15,-3);
        p.moveTo(7,4);p.lineTo(11,-3);p.lineTo(15,4);p.closeSubpath();
        line(11,4,11,16);line(11,16,0,16);
    } else {
        line(0,-32,0,-13);line(0,13,0,32);p.addEllipse(QPointF{},13,13);
        if (t=="dc") {line(-4,-5,4,-5);line(0,-9,0,-1);line(-4,6,4,6);}
        if (t=="pwm") {p.moveTo(-9,4);p.lineTo(-4,4);p.lineTo(-4,-5);p.lineTo(4,-5);p.lineTo(4,4);p.lineTo(9,4);}
        if (t=="sine") {p.moveTo(-10,0);p.cubicTo(-4,-13,4,13,10,0);}
        if (t=="voltmeter" || t=="ammeter") p.addText(-5,5,QApplication::font(),t=="voltmeter"?"V":"A");
    }
    return p;
}
// Return the shortest rectilinear route to any existing node on this net.
// Occupied corners cannot be crossed. At another net's straight segment only
// perpendicular travel is legal. The two nets remain electrically distinct.
QList<QPoint> route(QPoint start,const QSet<QPoint>& targets,const QSet<QPoint>& blocked,
                     const QHash<QPoint,int>& occupied,const QRect& bounds) {
    QQueue<QPoint> queue; queue.enqueue(start);
    QHash<QPoint,QPoint> parent; parent.insert(start,start);
    const QPoint dirs[]={{1,0},{0,1},{-1,0},{0,-1}};
    while (!queue.isEmpty()) {
        auto cur=queue.dequeue();
        if (targets.contains(cur)) {
            QList<QPoint> path{cur};
            while (cur!=start) {cur=parent.value(cur);path.prepend(cur);}
            return path;
        }
        for (QPoint d:dirs) {
            QPoint next=cur+d;
            if (!bounds.contains(next) || parent.contains(next) || (blocked.contains(next) && !targets.contains(next))) continue;
            int direction=d.x() ? 1 : 2;
            if ((occupied.value(cur)&direction) || (occupied.value(next)&direction)) continue;
            parent.insert(next,cur);queue.enqueue(next);
        }
    }
    return {};
}
}
void Sheet::redraw() {
    setRenderHint(QPainter::Antialiasing);setAcceptDrops(true);
    scene()->clear();
    QHash<QString,QPoint> pinPos;
    QSet<QPoint> blocked;
    QRect bounds(-12,-10,25,21);
    auto layout=circuit.document["layout"].toObject();
    auto parts=circuit.document["parts"].toObject();
    for (auto it=parts.begin();it!=parts.end();++it) {
        auto id=it.key(),type=it.value().toObject()["type"].toString();
        auto pos=layout[id].toObject();
        auto center=position(pos);int angle=pos["rotation"].toInt();
        bounds=bounds.united(QRect(center,QSize(1,1)));blocked.insert(center);
        auto* symbol=scene()->addPath(glyph(type),QPen(id==selected?QColor("#75418c"):QColor("#232832"),2));
        symbol->setData(0,id);symbol->setPos(center*pitch);symbol->setRotation(angle);symbol->setZValue(2);
        auto* label=scene()->addSimpleText(id);label->setPos(center*pitch+QPoint(17,-28));label->setData(0,id);label->setZValue(3);
        const auto pins=library().value(type).pins;
        auto delta=offsets(type);
        for (int i=0;i<pins.size();++i) {
            QString name=id+"."+pins[i];QPoint at=center+rotatePoint(delta[i],angle);
            blocked.insert(at);pinPos[name]=at;
            auto* dot=scene()->addEllipse(-4,-4,8,8,QPen(QColor("#764986")),name==pending?QBrush("#9457ae"):QBrush(Qt::white));
            dot->setPos(at*pitch);dot->setData(1,name);dot->setToolTip(name);dot->setZValue(4);
        }
    }
    bounds.adjust(-10,-8,10,8);
    QHash<QPoint,int> occupied;
    auto bends=circuit.document["bends"].toObject();
    auto nets=circuit.document["nets"].toObject();
    for (auto net=nets.begin();net!=nets.end();++net) {
        auto pins=net.value().toArray();
        if (pins.isEmpty()) continue;
        QSet<QPoint> tree{pinPos.value(pins.first().toString())};
        QHash<QPoint,int> directions,degree;
        for (int i=1;i<pins.size();++i) {
            QString pin=pins[i].toString();auto start=pinPos.value(pin);
            QList<QPoint> path;
            if (bends.contains(pin)) {
                QPoint bend=position(bends[pin].toObject());
                auto area=bounds.united(QRect(bend,QSize(1,1))).adjusted(-1,-1,1,1);
                auto first=route(start,{bend},blocked,occupied,area);
                if (!first.empty()) {
                    auto avoid=blocked;
                    for (auto p:first) if (p!=bend) avoid.insert(p);
                    auto second=route(bend,tree,avoid,occupied,area);
                    if (!second.empty()) {first.removeLast();path=first+second;}
                }
            } else path=route(start,tree,blocked,occupied,bounds);
            if (path.empty()) {
                auto* warn=scene()->addSimpleText("배선 경로 확인 · "+net.key());
                warn->setBrush(Qt::red);warn->setPos(start*pitch);continue;
            }
            for (int j=1;j<path.size();++j) {
                auto a=path[j-1],b=path[j];int axis=a.x()==b.x()?2:1;
                auto* wire=scene()->addLine(QLineF(a*pitch,b*pitch),QPen(QColor("#427268"),1.7));
                wire->setData(2,pin);wire->setToolTip(net.key());wire->setZValue(-1);
                tree.insert(a);tree.insert(b);directions[a]|=axis;directions[b]|=axis;
                degree[a]++;degree[b]++;
            }
        }
        for (auto p:tree) {
            occupied[p]|=directions.value(p);
            if (degree.value(p)>=3) {
                auto* dot=scene()->addEllipse(-3,-3,6,6,Qt::NoPen,QBrush("#427268"));dot->setPos(p*pitch);
            }
        }
    }
    setSceneRect(scene()->itemsBoundingRect().united(QRectF(-450,-300,900,600)).adjusted(-80,-80,80,80));
}
void Sheet::addPart(const QString& type,QPointF at) {
    QPoint p=grid(at);
    auto layout=circuit.document["layout"].toObject();
    QSet<QPoint> occupied;
    for (auto v:layout) {
        const QPoint center=position(v.toObject());
        for (int x=-2;x<=2;++x) for (int y=-2;y<=2;++y) occupied.insert(center+QPoint(x,y));
    }
    while (occupied.contains(p)) p+=QPoint(1,0);
    edit([&](Circuit& c){selected=c.insert(type,p.x(),p.y());});
}
void Sheet::mousePressEvent(QMouseEvent* e) {
    auto* item=itemAt(e->pos());
    if (!item) {pending.clear();selected.clear();redraw();return;}
    QString pin=item->data(1).toString();selected=item->data(0).toString();
    if (!pin.isEmpty()) {
        if (e->button()==Qt::RightButton) {pending.clear();edit([&](Circuit& c){c.disconnectPin(pin);});}
        else if (pending.isEmpty()) pending=pin;
        else {QString first=pending;pending.clear();edit([&](Circuit& c){c.connectPins(first,pin);});}
        redraw();return;
    }
    if (!selected.isEmpty()) {
        dragging=selected;
        auto pos=circuit.document["layout"].toObject()[selected].toObject();
        dragOffset=mapToScene(e->pos())-position(pos)*pitch;
    } else wirePin=item->data(2).toString();
    redraw();
}
void Sheet::mouseMoveEvent(QMouseEvent* e) {
    if (!dragging.isEmpty()) viewport()->setCursor(Qt::ClosedHandCursor);
    QGraphicsView::mouseMoveEvent(e);
}
void Sheet::mouseReleaseEvent(QMouseEvent* e) {
    QPoint p=grid(mapToScene(e->pos())-dragOffset);
    if (!dragging.isEmpty()) {
        auto id=dragging;dragging.clear();
        edit([&](Circuit& c){c.move(id,p.x(),p.y());});
    } else if (!wirePin.isEmpty() && (e->modifiers()&Qt::ShiftModifier)) {
        p=grid(mapToScene(e->pos()));
        edit([&](Circuit& c){auto bends=c.document["bends"].toObject();
            bends[wirePin]=QJsonObject{{"x",p.x()},{"y",p.y()}};c.document["bends"]=bends;});
    }
    wirePin.clear();viewport()->unsetCursor();
}
void Sheet::mouseDoubleClickEvent(QMouseEvent* e) {
    auto* item=itemAt(e->pos());
    if (!item) return;
    auto id=item->data(0).toString();if(id.isEmpty()) return;
    dragging.clear();
    auto parts=circuit.document["parts"].toObject(),part=parts[id].toObject(),params=part["parameters"].toObject();
    if (params.isEmpty()) return;
    bool ok=false;
    QString key=QInputDialog::getItem(this,id,"파라미터",params.keys(),0,false,&ok);if (!ok) return;
    auto s=QInputDialog::getText(this,id,key,QLineEdit::Normal,QString::number(params[key].toDouble(),'g',16),&ok);
    if (!ok) return;
    double v=s.toDouble(&ok);if(!ok || !std::isfinite(v)) return;
    edit([&](Circuit& c){params[key]=v;part["parameters"]=params;parts[id]=part;c.document["parts"]=parts;});
}
void Sheet::keyPressEvent(QKeyEvent* e) {
    if (e->key()==Qt::Key_Escape) {pending.clear();redraw();return;}
    if (!selected.isEmpty() && e->key()==Qt::Key_R) {edit([&](Circuit& c){c.rotate(selected);});return;}
    if (!selected.isEmpty() && e->key()==Qt::Key_Delete) {edit([&](Circuit& c){c.erase(selected);selected.clear();});return;}
    QGraphicsView::keyPressEvent(e);
}
void Sheet::dragEnterEvent(QDragEnterEvent* e) {if (qobject_cast<QListWidget*>(e->source())) e->acceptProposedAction();}
void Sheet::dragMoveEvent(QDragMoveEvent* e) {if (qobject_cast<QListWidget*>(e->source())) e->acceptProposedAction();}
void Sheet::dropEvent(QDropEvent* e) {
    auto* list=qobject_cast<QListWidget*>(e->source());
    if (list && list->currentItem()) {addPart(list->currentItem()->text(),mapToScene(e->position().toPoint()));e->acceptProposedAction();}
}
void Sheet::drawBackground(QPainter* p,const QRectF& rect) {
    p->fillRect(rect,Qt::white);p->setPen(QPen(QColor("#d7d9df"),1));
    for (int x=int(std::floor(rect.left()/pitch))*pitch;x<=rect.right();x+=pitch)
        for(int y=int(std::floor(rect.top()/pitch))*pitch;y<=rect.bottom();y+=pitch) p->drawPoint(x,y);
}
void Sheet::wheelEvent(QWheelEvent* e) {
    if (!(e->modifiers()&Qt::ControlModifier)) {QGraphicsView::wheelEvent(e);return;}
    setTransformationAnchor(QGraphicsView::AnchorUnderMouse);
    const double zoom=e->angleDelta().y()>0 ? 1.2 : 1/1.2;
    if (transform().m11()*zoom>=0.15 && transform().m11()*zoom<=6) scale(zoom,zoom);
    e->accept();
}
