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
#include <QToolTip>
#include <cmath>

namespace {
constexpr int pitch=32;
constexpr int jumpRadius=8;
constexpr int jumpClearance=4; // Includes the selected wire's stroke.
enum WireAxis { Horizontal=1, Vertical=2, BothAxes=Horizontal|Vertical };
QPoint grid(QPointF p) {return {qRound(p.x()/pitch),qRound(p.y()/pitch)};}
QPoint position(QJsonObject pos) {return {pos["x"].toInt(),pos["y"].toInt()};}
QPoint rotatePoint(QPoint p,int degrees) {
    for (int i=0;i<degrees;i+=90) p={-p.y(),p.x()};
    return p;
}
QList<QPoint> offsets(const QString& type) {
    if (type=="ground") return {{0,-1}};
    if (type=="nmos") return {{0,-1},{-1,0},{0,1}};
    if (type=="resistor" || type=="inductor" || type=="ammeter") return {{-1,0},{1,0}};
    if (type=="diode") return {{0,1},{0,-1}};
    return {{0,-1},{0,1}};
}
QPainterPath glyph(const QString& t) {
    QPainterPath p;
    auto line=[&](double x,double y,double a,double b){p.moveTo(x,y);p.lineTo(a,b);};
    auto diode=[&](double x){
        line(x-8,-7,x+8,-7);
        p.moveTo(x-7,7);p.lineTo(x,-7);p.lineTo(x+7,7);p.closeSubpath();
    };
    if (t=="resistor") {
        p.moveTo(-32,0);p.lineTo(-16,0);
        for (int k=0;k<6;++k) p.lineTo(-13+5*k,k%2 ? 5 : -5);
        p.lineTo(16,0);p.lineTo(32,0);
    } else if (t=="inductor") {
        p.moveTo(-32,0);p.lineTo(-26,0);
        for (int k=0;k<4;++k) p.arcTo(-26+13*k,-9,13,18,180,-180);
        p.lineTo(32,0);
    } else if (t=="capacitor") {
        line(0,-32,0,-4);line(-15,-4,15,-4);line(-15,4,15,4);line(0,4,0,32);
    } else if (t=="ground") {
        line(0,-32,0,-4);line(-12,-4,12,-4);line(-8,1,8,1);line(-3,6,3,6);
    } else if (t=="diode") {
        line(0,32,0,7);diode(0);line(0,-7,0,-32);
    } else if (t=="nmos") {
        line(-32,0,-22,0);line(-22,-24,-22,24);
        line(-16,-24,-16,-13);line(-16,-6,-16,6);line(-16,13,-16,24);
        line(0,-32,0,-20);line(0,20,0,32);
        line(-16,-20,13,-20);line(-16,20,13,20);
        // The substrate is tied to S; the diode's cathode is at D.
        line(-16,0,4,0);line(4,0,4,20);
        p.moveTo(-16,0);p.lineTo(-8,-4);p.lineTo(-8,4);p.closeSubpath();
        line(13,-20,13,-7);diode(13);line(13,7,13,20);
        p.addEllipse(QPointF(0,-20),1.4,1.4);p.addEllipse(QPointF(0,20),1.4,1.4);
        p.addEllipse(QPointF(4,20),1.4,1.4);
    } else if (t=="dc") {
        line(0,-32,0,-4);line(-15,-4,15,-4);
        line(-8,4,8,4);line(0,4,0,32);
    } else if (t=="ammeter") {
        p.addEllipse(QPointF{},20,20);line(-32,0,-20,0);line(20,0,32,0);
        // Follow the core's positive current reference: p on the left, n on the right.
        line(-11,8,10,8);line(10,8,5,4);line(10,8,5,12);
    } else {
        line(0,-32,0,-20);line(0,20,0,32);p.addEllipse(QPointF{},20,20);
        if (t=="pwm") {p.moveTo(-13,6);p.lineTo(-7,6);p.lineTo(-7,-6);p.lineTo(7,-6);p.lineTo(7,6);p.lineTo(13,6);}
        if (t=="sine") {p.moveTo(-14,0);p.cubicTo(-10,-15,-5,-15,0,0);p.cubicTo(5,15,10,15,14,0);}
        if (t=="sink") {line(0,-11,0,10);line(0,10,-5,4);line(0,10,5,4);}
    }
    return p;
}
QString valueLabel(const QString& type,const QJsonObject& values) {
    auto unit=[](double value,const QString& symbol){
        const double mag=std::abs(value);
        if(mag>=1000) return QString::number(value/1e3,'g',4)+" k"+symbol;
        if(mag>0 && mag<1e-3) return QString::number(value*1e6,'g',4)+" µ"+symbol;
        if(mag>0 && mag<1) return QString::number(value*1e3,'g',4)+" m"+symbol;
        return QString::number(value,'g',4)+" "+symbol;
    };
    if(type=="inductor") return unit(values["inductance_H"].toDouble(),"H");
    if(type=="capacitor") return unit(values["capacitance_F"].toDouble(),"F");
    if(type=="resistor") return unit(values["resistance_ohm"].toDouble(),"Ω");
    if(type=="dc") return unit(values["voltage_V"].toDouble(),"V");
    if(type=="sine") return unit(values["amplitude_V"].toDouble()/std::sqrt(2.),"V RMS");
    if(type=="sink") return unit(values["current_A"].toDouble(),"A");
    if(type=="pwm") return unit(values["frequency_Hz"].toDouble(),"Hz");
    return {};
}
struct Caption {
    QString text;
    QPointF offset;
    int size;
    bool bold;
    bool followsSelection=true;
    QFont font() const {
        auto f=QApplication::font(); f.setPixelSize(size); f.setBold(bold); return f;
    }
    QRectF bounds() const {
        QGraphicsSimpleTextItem item(text); item.setFont(font());
        return item.boundingRect().translated(offset);
    }
};

QList<Caption> captions(const Circuit& circuit,const QString& id,const QJsonObject& part,int angle) {
    constexpr int valueLabelSize=13;
    const auto type=part["type"].toString();
    bool horizontal=type=="inductor" || type=="resistor" || type=="ammeter";
    if(angle%180) horizontal=!horizontal;
    QPoint name=horizontal ? QPoint(-16,-36) : QPoint(23,-16);
    if(type=="ammeter" && horizontal) name.ry()-=4;
    QList<Caption> labels{{id,name,16,true}};
    const auto value=valueLabel(type,part["parameters"].toObject());
    if(!value.isEmpty()) labels.append({value,horizontal ? QPoint(-24,6) : name+QPoint(0,20),valueLabelSize,false});
    // Rotate terminal-relative anchors and keep the lettering upright.
    auto mark=[&](const QString& text,QPoint anchor,int size=14,bool bold=false,bool outsideGate=false) {
        Caption caption{text,{},size,bold,false};
        const auto rect=caption.bounds();
        caption.offset=rotatePoint(anchor,angle)-rect.center();
        if(outsideGate) {
            const QPoint outward=rotatePoint(QPoint(-1,0),angle);
            caption.offset+=QPointF(outward.x()*rect.width()/2,outward.y()*rect.height()/2);
        }
        labels.append(caption);
    };
    if(type=="nmos") {
        const auto pins=offsets(type);
        mark("D",pins[0]*pitch+QPoint(15,-13),valueLabelSize);
        mark("G",pins[1]*pitch+QPoint(-2,17),valueLabelSize);
        mark("S",pins[2]*pitch+QPoint(15,13),valueLabelSize);
        const auto signal=circuit.gateSignal(id+".g");
        if(!signal.isEmpty()) mark("제어 "+signal,{-46,14},14,false,true);
        // On rotation the name follows the diode side, clear of the gate lead.
        auto& label=labels.first();
        const auto rect=label.bounds();
        if(angle==0) label.offset.setX(27);
        if(angle==90) label.offset={-rect.width()/2,27};
        if(angle==180) label.offset={-27-rect.width(),-16};
        if(angle==270) label.offset={-rect.width()/2,-27-rect.height()};
    } else if(type=="voltmeter") {
        const auto pins=offsets(type);
        mark("V",{0,0});
        mark("+",pins[0]*pitch+QPoint(-27,12),22,true);
        mark("−",pins[1]*pitch+QPoint(-27,-12),22,true);
    } else if(type=="ammeter") {
        mark("A",{0,-8});
    }
    return labels;
}

QList<QRectF> footprint(const Circuit& circuit,const QString& id,const QJsonObject& part,int angle,QPoint center) {
    QTransform turn;
    turn.rotate(angle);
    const QPoint origin=center*pitch;
    QList<QRectF> boxes{turn.mapRect(glyph(part["type"].toString()).boundingRect())
                             .translated(origin).adjusted(-3,-3,3,3)};
    for(const auto& label:captions(circuit,id,part,angle))
        boxes.append(label.bounds().translated(origin).adjusted(-2,-2,2,2));
    return boxes;
}

QString collision(const Circuit& c,const QString& id,QPoint at,int angle) {
    const auto parts=c.document["parts"].toObject(),layout=c.document["layout"].toObject();
    const auto moving=footprint(c,id,parts[id].toObject(),angle,at);
    for(auto it=parts.begin();it!=parts.end();++it) {
        if(it.key()==id) continue;
        const auto pos=layout[it.key()].toObject();
        const auto fixed=footprint(c,it.key(),it.value().toObject(),pos["rotation"].toInt(),position(pos));
        for(const auto& a:moving) for(const auto& b:fixed) if(a.intersects(b)) return it.key();
    }
    return {};
}

using Edges = QSet<QPair<QPoint,QPoint>>;
void reserveJumpSites(const QRectF& obstacle,QSet<QPoint>& sites) {
    // Mirror the clearance envelope about the candidate grid point.
    constexpr int rim=jumpRadius+jumpClearance;
    const auto candidates=obstacle.adjusted(-rim,-jumpClearance,rim,rim);
    for(int x=int(std::ceil(candidates.left()/pitch));x<=std::floor(candidates.right()/pitch);++x)
        for(int y=int(std::ceil(candidates.top()/pitch));y<=std::floor(candidates.bottom()/pitch);++y)
            sites.insert({x,y});
}

void reserveLabelEdges(const QRectF& rect,Edges& edges) {
    // A label may cover the middle of a grid edge without containing either endpoint.
    for(int x=int(std::floor(rect.left()/pitch))-1;x<=std::ceil(rect.right()/pitch);++x)
        for(int y=int(std::floor(rect.top()/pitch))-1;y<=std::ceil(rect.bottom()/pitch);++y) {
            const QPoint a(x,y);
            for(auto dir:{QPoint(1,0),QPoint(0,1)}) {
                const QPoint b=a+dir;
                const auto wire=QRectF(a*pitch,b*pitch).normalized().adjusted(-1.5,-1.5,1.5,1.5);
                if(rect.intersects(wire)) {edges.insert({a,b});edges.insert({b,a});}
            }
        }
}

// Return the shortest rectilinear route to any existing node on this net.
// Only the interior of another net's straight run can be crossed, and only
// where the jump fits. A path can neither begin nor end at a crossing.
QList<QPoint> route(QPoint start,const QSet<QPoint>& targets,const QSet<QPoint>& blocked,
                     const QHash<QPoint,int>& occupied,const QRect& bounds,const Edges& labels) {
    if(occupied.contains(start)) return {};
    QQueue<QPoint> queue; queue.enqueue(start);
    QHash<QPoint,QPoint> parent; parent.insert(start,start);
    const QPoint dirs[]={{1,0},{0,1},{-1,0},{0,-1}};
    while (!queue.isEmpty()) {
        auto cur=queue.dequeue();
        if (targets.contains(cur) && !occupied.contains(cur)) {
            QList<QPoint> path{cur};
            while (cur!=start) {cur=parent.value(cur);path.prepend(cur);}
            return path;
        }
        for (QPoint d:dirs) {
            QPoint next=cur+d;
            if (labels.contains({cur,next}) || !bounds.contains(next) || parent.contains(next) || (blocked.contains(next) && !targets.contains(next))) continue;
            int direction=d.x() ? Horizontal : Vertical;
            if ((occupied.value(cur)&direction) || (occupied.value(next)&direction)) continue;
            parent.insert(next,cur);queue.enqueue(next);
        }
    }
    return {};
}
}
void Sheet::redraw() {
    setRenderHint(QPainter::Antialiasing);setAcceptDrops(true);
    const bool hadContent=!scene()->items().empty();
    const auto oldCenter=mapToScene(viewport()->rect().center());
    scene()->clear();
    QHash<QString,QPoint> pinPos;
    Edges labelEdges;
    QSet<QPoint> blocked,jumpBlocked;
    QRect bounds(-12,-10,25,21);
    const auto& document=circuit.document;
    const auto layout=document["layout"].toObject(),parts=document["parts"].toObject();
    for (auto it=parts.begin();it!=parts.end();++it) {
        auto id=it.key(),type=it.value().toObject()["type"].toString();
        auto pos=layout[id].toObject();
        auto center=position(pos);int angle=pos["rotation"].toInt();
        bounds=bounds.united(QRect(center,QSize(1,1)));blocked.insert(center);
        const QColor ink=id==selected ? QColor("#007d72") : QColor(Qt::black);
        auto* symbol=scene()->addPath(glyph(type),QPen(ink,2,Qt::SolidLine,Qt::RoundCap,Qt::RoundJoin),
            type=="nmos" || type=="diode" ? QBrush(ink):QBrush(Qt::NoBrush));
        symbol->setData(0,id);symbol->setPos(center*pitch);symbol->setRotation(angle);symbol->setZValue(2);
        reserveJumpSites(symbol->sceneBoundingRect(),jumpBlocked);
        if (id==selected) {
            QPainterPath halo;halo.addRoundedRect(-40,-31,100,62,6,6);
            auto* h=scene()->addPath(halo,QPen(QColor("#259e92"),1,Qt::DashLine),QBrush("#def4ee"));
            h->setPos(center*pitch);h->setZValue(0);h->setData(0,id);
        }
        for(const auto& caption:captions(circuit,id,it.value().toObject(),angle)) {
            auto* label=scene()->addSimpleText(caption.text,caption.font());
            label->setBrush(caption.followsSelection && caption.bold ? ink : QColor(Qt::black));
            label->setPos(center*pitch+caption.offset);label->setData(0,id);label->setZValue(3);
            const QFontMetricsF metrics(label->font());
            const auto inkBounds=metrics.tightBoundingRect(caption.text)
                .translated(label->scenePos()+QPointF(0,metrics.ascent()));
            reserveLabelEdges(inkBounds,labelEdges);
            reserveJumpSites(inkBounds,jumpBlocked);
        }
        const auto pins=library().value(type).pins;
        auto delta=offsets(type);
        for (int i=0;i<pins.size();++i) {
            QString name=id+"."+pins[i];QPoint at=center+rotatePoint(delta[i],angle);
            blocked.insert(at);pinPos[name]=at;
            bool visible=id==selected || !pending.isEmpty();
            auto* dot=scene()->addRect(-4,-4,8,8,visible?QPen(QColor("#168c80")):QPen(Qt::NoPen),
                visible?QBrush(name==pending?QColor("#168c80"):QColor(Qt::white)):QBrush(Qt::transparent));
            dot->setPos(at*pitch);dot->setData(1,name);dot->setToolTip(name);dot->setZValue(4);
        }
    }
    auto addAnnotation=[&](const QString& text,QPoint at,int size) {
        auto font=QApplication::font();font.setBold(true);font.setPixelSize(size);
        auto* label=scene()->addSimpleText(text,font);
        label->setBrush(Qt::black);label->setPos(at*pitch);
        reserveLabelEdges(label->sceneBoundingRect(),labelEdges);
        reserveJumpSites(label->sceneBoundingRect(),jumpBlocked);
    };
    for(auto entry:document["annotations"].toArray()) {
        const auto a=entry.toObject();
        addAnnotation(a["text"].toString(),position(a),16);
    }
    if(document["title"].isString()) addAnnotation(document["title"].toString(),{0,-4},25);
    bounds.adjust(-10,-8,10,8);
    QHash<QPoint,int> occupied;
    QSet<QPoint> crossings;
    QList<QGraphicsLineItem*> wires;
    const auto bends=document["bends"].toObject();
    const auto trunks=document["trunks"].toObject();
    const auto nets=document["nets"].toObject();
    QMap<QString,NetRoute> nextRoutes;
    QStringList keep,reroute;
    auto obstructsJump=[&](QPoint p){return jumpBlocked.contains(p) && routeCrossings.contains(p);};
    for(auto net=nets.begin();net!=nets.end();++net) {
        auto& current=nextRoutes[net.key()];
        current.pins=net.value().toArray();
        current.trunk=trunks.value(net.key()).toArray();
        QSet<QPoint> terminals;
        for(auto ref:current.pins) {
            const auto pin=ref.toString();
            current.terminals[pin]=pinPos.value(pin);
            terminals.insert(pinPos.value(pin));
            if(bends.contains(pin)) current.bends.insert(pin,bends.value(pin));
        }
        const auto old=routes.constFind(net.key());
        bool reusable=old!=routes.cend() && old->pins==current.pins && old->trunk==current.trunk &&
                      old->terminals==current.terminals && old->bends==current.bends;
        if(reusable) for(const auto& segment:old->segments) {
            if(labelEdges.contains({segment.a,segment.b}) ||
               obstructsJump(segment.a) || obstructsJump(segment.b) ||
               (blocked.contains(segment.a) && !terminals.contains(segment.a)) ||
               (blocked.contains(segment.b) && !terminals.contains(segment.b))) {
                reusable=false;break;
            }
        }
        if(reusable) keep.append(net.key());
        else reroute.append(net.key());
    }
    // Reserve unaffected routes before searching for paths around edited components.
    const auto order=keep+reroute;
    for (const auto& name:order) {
        auto& cached=nextRoutes[name];
        auto pins=cached.pins;
        if (pins.isEmpty()) continue;
        QSet<QPoint> terminals;
        for(auto pin:pins) terminals.insert(pinPos[pin.toString()]);
        QSet<QPoint> tree{pinPos.value(pins.first().toString())};
        QHash<QPoint,int> directions,degree;
        auto edge=[&](QPoint a,QPoint b,const QString& pin){
            int axis=a.x()==b.x() ? Vertical : Horizontal;
            const bool active=activeNet==name;
            auto* wire=scene()->addLine(QLineF(a*pitch,b*pitch),QPen(active?QColor("#007d72"):QColor(Qt::black),active?3:1.8));
            wire->setData(2,pin);wire->setData(3,name);wire->setToolTip(name);wire->setZValue(-1);
            wires.append(wire);
            cached.segments.append({a,b,pin});
            tree.insert(a);tree.insert(b);directions[a]|=axis;directions[b]|=axis;
            degree[a]++;degree[b]++;
        };
        const bool reused=keep.contains(name);
        if(reused) for(const auto& segment:routes.constFind(name)->segments)
            edge(segment.a,segment.b,segment.pin);
        auto trunk=cached.trunk;
        bool fixed=false;QPoint trunkStart,trunkEnd;
        if (!reused && trunk.size()==2 && trunk[0].toArray().size()==2 && trunk[1].toArray().size()==2) {
            auto a=trunk[0].toArray(),b=trunk[1].toArray();
            QPoint begin(a[0].toInt(),a[1].toInt()),end(b[0].toInt(),b[1].toInt());
            QPoint delta=end-begin;
            int length=delta.manhattanLength();
            if(length>0 && length<1000 && (delta.x()==0 || delta.y()==0)) {
                QPoint dir=delta/length;
                const int axis=dir.x() ? Horizontal : Vertical;
                fixed=true;
                for(int k=0;k<=length;++k) {
                    auto p=begin+dir*k;
                    if(k<length && labelEdges.contains({p,p+dir})) fixed=false;
                    if((blocked.contains(p) && !terminals.contains(p)) || (occupied.value(p)&axis)) fixed=false;
                    if(occupied.contains(p) && (k==0 || k==length)) fixed=false;
                }
                if(fixed) {
                    trunkStart=begin;trunkEnd=end;
                    tree.clear();
                    for(int k=0;k<length;++k) edge(begin+dir*k,begin+dir*(k+1),{});
                }
            }
        }
        bool complete=true;
        for (int i=fixed?0:1;!reused && i<pins.size();++i) {
            QString pin=pins[i].toString();auto start=pinPos.value(pin);
            if(tree.contains(start) && !occupied.contains(start) && !bends.contains(pin)) continue;
            QSet<QPoint> targets=tree;
            auto avoid=blocked;
            // A branch must stop at the tree before it can reach another net there.
            for(auto p:tree) if(occupied.contains(p)) {
                avoid.insert(p);
                targets.remove(p);
            }
            if(fixed) {
                QPoint tap(std::clamp(start.x(),std::min(trunkStart.x(),trunkEnd.x()),std::max(trunkStart.x(),trunkEnd.x())),
                           std::clamp(start.y(),std::min(trunkStart.y(),trunkEnd.y()),std::max(trunkStart.y(),trunkEnd.y())));
                if(targets.contains(tap)) targets={tap};
                avoid.unite(tree);
            }
            QList<QPoint> path;
            if (bends.contains(pin)) {
                QPoint bend=position(bends[pin].toObject());
                auto area=bounds.united(QRect(bend,QSize(1,1))).adjusted(-1,-1,1,1);
                auto approach=avoid;
                approach.unite(tree);
                auto first=route(start,{bend},approach,occupied,area,labelEdges);
                if (!first.empty() && (!tree.contains(bend) || targets.contains(bend))) {
                    for (auto p:first) if (p!=bend) avoid.insert(p);
                    auto second=route(bend,targets,avoid,occupied,area,labelEdges);
                    if (!second.empty()) {first.removeLast();path=first+second;}
                }
            } else path=route(start,targets,avoid,occupied,bounds,labelEdges);
            if (path.empty()) {
                complete=false;
                auto* warn=scene()->addSimpleText("배선 경로 확인 · "+name);
                warn->setBrush(Qt::red);warn->setPos(start*pitch);continue;
            }
            for (int j=1;j<path.size();++j) {
                edge(path[j-1],path[j],pin);
            }
        }
        for (auto p:tree) {
            if(!degree.contains(p)) continue;
            const int axis=directions.value(p);
            const bool terminal=terminals.contains(p);
            const bool straight=degree.value(p)==2 && axis!=BothAxes && !terminal && !jumpBlocked.contains(p);
            if(straight && occupied.value(p)==(BothAxes^axis)) crossings.insert(p);
            occupied[p]|=straight ? axis : BothAxes;
            if (degree.value(p)+int(terminal)>=3) {
                auto* dot=scene()->addEllipse(-3,-3,6,6,Qt::NoPen,QBrush(Qt::black));dot->setPos(p*pitch);
            }
        }
        if(!complete) nextRoutes.remove(name);
    }
    routes=std::move(nextRoutes);
    routeCrossings=crossings;
    for(auto* wire:wires) {
        auto line=wire->line();
        auto trim=[&](QPointF at,QPointF other) {
            if(!crossings.contains(grid(at))) return at;
            if(at.x()==other.x()) {
                // Separate the vertical stroke from the entire jump.
                return at+QPointF(0,other.y()<at.y() ? -jumpRadius-jumpClearance : jumpClearance);
            }
            if(other.x()<at.x()) {
                const QRectF rect(at.x()-jumpRadius,at.y()-jumpRadius,2*jumpRadius,2*jumpRadius);
                QPainterPath jump;
                jump.arcMoveTo(rect,180);
                jump.arcTo(rect,180,-180);
                auto* arc=scene()->addPath(jump,wire->pen());
                arc->setData(2,wire->data(2));arc->setData(3,wire->data(3));
                arc->setToolTip(wire->toolTip());arc->setZValue(wire->zValue());
            }
            return at+QPointF(other.x()<at.x() ? -jumpRadius : jumpRadius,0);
        };
        wire->setLine(QLineF(trim(line.p1(),line.p2()),trim(line.p2(),line.p1())));
    }
    auto content=scene()->itemsBoundingRect();
    setSceneRect(content.isEmpty()?QRectF(-450,-300,900,600):content.adjusted(-80,-80,80,80));
    if(hadContent) centerOn(oldCenter);
}
void Sheet::addPart(const QString& type,QPointF at) {
    QPoint p=grid(at);
    edit([&](Circuit& c){
        selected=c.insert(type,p.x(),p.y());
        while(!collision(c,selected,p,0).isEmpty()) ++p.rx();
        c.move(selected,p.x(),p.y());
    });
}
void Sheet::mousePressEvent(QMouseEvent* e) {
    auto* item=itemAt(e->pos());
    activeNet.clear();
    if (!item) {pending.clear();selected.clear();redraw();return;}
    QString pin=item->data(1).toString();selected=item->data(0).toString();
    if (!pin.isEmpty()) {
        if (e->button()==Qt::RightButton) {pending.clear();edit([&](Circuit& c){c.disconnectPin(pin);});}
        else if (pending.isEmpty()) pending=pin;
        else {QString first=pending;pending.clear();
            try {edit([&](Circuit& c){c.connectPins(first,pin);});}
            catch(const std::exception& err){QToolTip::showText(e->globalPosition().toPoint(),err.what(),this);}
        }
        redraw();return;
    }
    if (!selected.isEmpty()) {
        dragging=selected;
        auto pos=circuit.document["layout"].toObject()[selected].toObject();
        dragOffset=mapToScene(e->pos())-position(pos)*pitch;
    } else {wirePin=item->data(2).toString();activeNet=item->data(3).toString();}
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
        int angle=circuit.document["layout"].toObject()[id].toObject()["rotation"].toInt();
        QString other=collision(circuit,id,p,angle);
        if(other.isEmpty()) edit([&](Circuit& c){c.move(id,p.x(),p.y());});
        else QToolTip::showText(e->globalPosition().toPoint(),other+" 부품과 겹칩니다.",this);
    } else if (!wirePin.isEmpty() && (e->modifiers()&Qt::ShiftModifier)) {
        p=grid(mapToScene(e->pos()));
        edit([&](Circuit& c){c.setWireBend(wirePin,p.x(),p.y());});
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
    edit([&](Circuit& c){c.setParameter(id,key,v);});
}
void Sheet::keyPressEvent(QKeyEvent* e) {
    if (e->key()==Qt::Key_Escape) {pending.clear();redraw();return;}
    if (!selected.isEmpty() && e->key()==Qt::Key_R) {
        auto pos=circuit.document["layout"].toObject()[selected].toObject();
        auto other=collision(circuit,selected,position(pos),(pos["rotation"].toInt()+90)%360);
        if(other.isEmpty()) edit([&](Circuit& c){c.rotate(selected);});
        else QToolTip::showText(QCursor::pos(),other+" 부품과 겹칩니다.",this);
        return;
    }
    if (!selected.isEmpty() && e->key()==Qt::Key_Delete) {edit([&](Circuit& c){c.erase(selected);selected.clear();});return;}
    QGraphicsView::keyPressEvent(e);
}
void Sheet::dragEnterEvent(QDragEnterEvent* e) {if (library().contains(QString::fromUtf8(e->mimeData()->data(partMime)))) e->acceptProposedAction();}
void Sheet::dragMoveEvent(QDragMoveEvent* e) {if (library().contains(QString::fromUtf8(e->mimeData()->data(partMime)))) e->acceptProposedAction();}
void Sheet::dropEvent(QDropEvent* e) {
    const auto type=QString::fromUtf8(e->mimeData()->data(partMime));
    if(library().contains(type)){addPart(type,mapToScene(e->position().toPoint()));e->acceptProposedAction();}
}
void Sheet::drawBackground(QPainter* p,const QRectF& rect) {
    p->fillRect(rect,QColor("#fbfcfa"));p->setPen(QPen(QColor("#d7e1df"),1.3));
    const int interval=pitch/2;
    for (int x=int(std::floor(rect.left()/interval))*interval;x<=rect.right();x+=interval)
        for(int y=int(std::floor(rect.top()/interval))*interval;y<=rect.bottom();y+=interval) p->drawPoint(x,y);
}
void Sheet::wheelEvent(QWheelEvent* e) {
    if (!(e->modifiers()&Qt::ControlModifier)) {QGraphicsView::wheelEvent(e);return;}
    setTransformationAnchor(QGraphicsView::AnchorUnderMouse);
    const double zoom=e->angleDelta().y()>0 ? 1.2 : 1/1.2;
    if (transform().m11()*zoom>=0.15 && transform().m11()*zoom<=6) scale(zoom,zoom);
    e->accept();
}
