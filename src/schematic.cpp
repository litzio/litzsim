#include "studio.h"
#include <QApplication>
#include <QDragEnterEvent>
#include <QDropEvent>
#include <QFileInfo>
#include <QGraphicsItem>
#include <QGraphicsScene>
#include <QInputDialog>
#include <QJsonArray>
#include <QKeyEvent>
#include <QMimeData>
#include <QMouseEvent>
#include <QPainter>
#include <QPdfWriter>
#include <QQueue>
#include <QRawFont>
#include <QSaveFile>
#include <QSet>
#include <QSvgGenerator>
#include <QTextLayout>
#include <QWheelEvent>
#include <QToolTip>
#include <algorithm>
#include <cmath>
#include <limits>

namespace {
constexpr int pitch=32;
constexpr int jumpRadius=8;
constexpr int jumpClearance=4;
const QColor sheetBackground("#fbfcfa");
constexpr int displayRole=4;
enum DisplayRole { EditorOnly=1, JumpCasing=2 };
constexpr int labelRole=5;
constexpr int labelOrigin=6;
constexpr int sectionPixels=16;
constexpr int titlePixels=25;
enum class LabelRole { Meter, Polarity, Terminal, Id, Value, Gate, Section, Title, Warning };

double publicationPoints(LabelRole role) {
    switch(role) {
    case LabelRole::Id:
    case LabelRole::Polarity:
    case LabelRole::Section: return 9;
    case LabelRole::Title: return 9.0*titlePixels/sectionPixels;
    default: return 8;
    }
}

class ExportLabel : public QGraphicsSimpleTextItem {
public:
    enum { Type=UserType+1 };
    int type() const override { return Type; }
    explicit ExportLabel(const QGraphicsSimpleTextItem& source)
        :QGraphicsSimpleTextItem(source.text()), role(LabelRole(source.data(labelRole).toInt())),
          anchor(source.sceneBoundingRect().center()),owner(source.data(0).toString()),
          origin(source.data(labelOrigin).toPointF()) {
        setFont(source.font());
        setBrush(Qt::black);
        setTransformOriginPoint(boundingRect().center());
        QTextLayout layout(QString(text()).replace(u'\n',QChar::LineSeparator),font());
        layout.beginLayout();
        qreal y=0;
        for(auto line=layout.createLine();line.isValid();line=layout.createLine()) {
            line.setLineWidth(std::numeric_limits<int>::max());
            line.setPosition({0,y});
            y+=line.height();
        }
        layout.endLayout();
        outline.setFillRule(Qt::WindingFill);
        for(const auto& run:layout.glyphRuns()) {
            const auto glyphs=run.glyphIndexes();
            const auto positions=run.positions();
            const auto font=run.rawFont();
            for(qsizetype i=0;i<glyphs.size();++i)
                outline.addPath(font.pathForGlyph(glyphs[i]).translated(positions[i]));
        }
    }
    QRectF inkBounds() const { return mapRectToScene(outline.boundingRect()); }
    const LabelRole role;
    const QPointF anchor;
    const QString owner;
    const QPointF origin;
    void paint(QPainter* painter,const QStyleOptionGraphicsItem* option,QWidget* widget) override {
        if(!useOutline) {
            QGraphicsSimpleTextItem::paint(painter,option,widget);
            return;
        }
        painter->setPen(Qt::NoPen);
        painter->setBrush(brush());
        painter->drawPath(outline);
    }
    bool useOutline=false;
private:
    QPainterPath outline;
};
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
    LabelRole role;
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
    QList<Caption> labels{{id,name,16,true,LabelRole::Id}};
    const auto value=valueLabel(type,part["parameters"].toObject());
    if(!value.isEmpty()) labels.append({value,horizontal ? QPoint(-24,6) : name+QPoint(0,20),
                                        valueLabelSize,false,LabelRole::Value});
    // Rotate terminal-relative anchors and keep the lettering upright.
    auto mark=[&](const QString& text,QPoint anchor,LabelRole role,int size=14,bool bold=false) {
        Caption caption{text,{},size,bold,role};
        const auto rect=caption.bounds();
        caption.offset=rotatePoint(anchor,angle)-rect.center();
        if(role==LabelRole::Gate) {
            const QPoint outward=rotatePoint(QPoint(-1,0),angle);
            caption.offset+=QPointF(outward.x()*rect.width()/2,outward.y()*rect.height()/2);
        }
        labels.append(caption);
    };
    if(type=="nmos") {
        const auto pins=offsets(type);
        mark("D",pins[0]*pitch+QPoint(15,-13),LabelRole::Terminal,valueLabelSize);
        mark("G",pins[1]*pitch+QPoint(-2,17),LabelRole::Terminal,valueLabelSize);
        mark("S",pins[2]*pitch+QPoint(15,13),LabelRole::Terminal,valueLabelSize);
        const auto signal=circuit.gateSignal(id+".g");
        if(!signal.isEmpty()) mark(signal,{-46,14},LabelRole::Gate);
        // On rotation the name follows the diode side, clear of the gate lead.
        auto& label=labels.first();
        const auto rect=label.bounds();
        if(angle==0) label.offset.setX(27);
        if(angle==90) label.offset={-rect.width()/2,27};
        if(angle==180) label.offset={-27-rect.width(),-16};
        if(angle==270) label.offset={-rect.width()/2,-27-rect.height()};
    } else if(type=="voltmeter") {
        const auto pins=offsets(type);
        mark("V",{0,0},LabelRole::Meter);
        mark("+",pins[0]*pitch+QPoint(-27,12),LabelRole::Polarity,22,true);
        mark("−",pins[1]*pitch+QPoint(-27,-12),LabelRole::Polarity,22,true);
    } else if(type=="ammeter") {
        mark("A",{0,-8},LabelRole::Meter);
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
    drawSchematic(scene(),circuit,routes,routeCrossings,selected,pending,activeNet);
    auto content=scene()->itemsBoundingRect();
    setSceneRect(content.isEmpty()?QRectF(-450,-300,900,600):content.adjusted(-80,-80,80,80));
    if(hadContent) centerOn(oldCenter);
}

void Sheet::drawSchematic(QGraphicsScene* scene,const Circuit& circuit,QMap<QString,NetRoute>& routes,
                          QSet<QPoint>& routeCrossings,const QString& selected,const QString& pending,
                          const QString& activeNet) {
    scene->clear();
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
        auto* symbol=scene->addPath(glyph(type),QPen(ink,2,Qt::SolidLine,Qt::RoundCap,Qt::RoundJoin),
            type=="nmos" || type=="diode" ? QBrush(ink):QBrush(Qt::NoBrush));
        symbol->setData(0,id);symbol->setPos(center*pitch);symbol->setRotation(angle);symbol->setZValue(2);
        reserveJumpSites(symbol->sceneBoundingRect(),jumpBlocked);
        if (id==selected) {
            QPainterPath halo;halo.addRoundedRect(-40,-31,100,62,6,6);
            auto* h=scene->addPath(halo,QPen(QColor("#259e92"),1,Qt::DashLine),QBrush("#def4ee"));
            h->setPos(center*pitch);h->setZValue(0);h->setData(0,id);
            h->setData(displayRole,EditorOnly);
        }
        for(const auto& caption:captions(circuit,id,it.value().toObject(),angle)) {
            auto* label=scene->addSimpleText(caption.text,caption.font());
            label->setBrush(caption.role==LabelRole::Id ? ink : QColor(Qt::black));
            label->setPos(center*pitch+caption.offset);label->setData(0,id);label->setZValue(3);
            label->setData(labelRole,int(caption.role));
            label->setData(labelOrigin,QPointF(center*pitch));
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
            auto* dot=scene->addRect(-4,-4,8,8,visible?QPen(QColor("#168c80")):QPen(Qt::NoPen),
                visible?QBrush(name==pending?QColor("#168c80"):QColor(Qt::white)):QBrush(Qt::transparent));
            dot->setPos(at*pitch);dot->setData(1,name);dot->setToolTip(name);dot->setZValue(4);
            dot->setData(displayRole,EditorOnly);
        }
    }
    auto addAnnotation=[&](const QString& text,QPoint at,int size,LabelRole role) {
        auto font=QApplication::font();font.setBold(true);font.setPixelSize(size);
        auto* label=scene->addSimpleText(text,font);
        label->setBrush(Qt::black);label->setPos(at*pitch);
        label->setData(labelRole,int(role));
        reserveLabelEdges(label->sceneBoundingRect(),labelEdges);
        reserveJumpSites(label->sceneBoundingRect(),jumpBlocked);
    };
    for(auto entry:document["annotations"].toArray()) {
        const auto a=entry.toObject();
        addAnnotation(a["text"].toString(),position(a),sectionPixels,LabelRole::Section);
    }
    if(document["title"].isString())
        addAnnotation(document["title"].toString(),{0,-4},titlePixels,LabelRole::Title);
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
            auto* wire=scene->addLine(QLineF(a*pitch,b*pitch),QPen(active?QColor("#007d72"):QColor(Qt::black),active?3:1.8));
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
                auto* warn=scene->addSimpleText("배선 경로 확인 · "+name);
                warn->setData(labelRole,int(LabelRole::Warning));
                warn->setBrush(Qt::red);warn->setPos(start*pitch);continue;
            }
            for (int j=1;j<path.size();++j) {
                edge(path[j-1],path[j],pin);
            }
        }
        QList<QPoint> junctions;
        for (auto p:tree) {
            if(!degree.contains(p)) continue;
            const int axis=directions.value(p);
            const bool terminal=terminals.contains(p);
            const bool straight=degree.value(p)==2 && axis!=BothAxes && !terminal && !jumpBlocked.contains(p);
            if(straight && occupied.value(p)==(BothAxes^axis)) crossings.insert(p);
            occupied[p]|=straight ? axis : BothAxes;
            if (degree.value(p)+int(terminal)>=3) junctions.append(p);
        }
        std::sort(junctions.begin(),junctions.end(),[](QPoint a,QPoint b){
            return a.y()==b.y() ? a.x()<b.x() : a.y()<b.y();
        });
        for(auto p:junctions) {
            auto* dot=scene->addEllipse(-3,-3,6,6,Qt::NoPen,QBrush(Qt::black));dot->setPos(p*pitch);
        }
        if(!complete) nextRoutes.remove(name);
    }
    routes=std::move(nextRoutes);
    routeCrossings=crossings;
    for(auto* wire:wires) {
        auto line=wire->line();
        auto trim=[&](QPointF at,QPointF other) {
            if(at.x()==other.x() || !crossings.contains(grid(at))) return at;
            if(other.x()<at.x()) {
                const QRectF rect(at.x()-jumpRadius,at.y()-jumpRadius,2*jumpRadius,2*jumpRadius);
                QPainterPath jump;
                jump.arcMoveTo(rect,180);
                jump.arcTo(rect,180,-180);
                // A short backing stroke gives the overpass clearance at its crown.
                // Both vertical segments still reach the crossing grid point.
                QPainterPath crown;
                crown.arcMoveTo(rect,120);
                crown.arcTo(rect,120,-60);
                auto* casing=scene->addPath(crown,QPen(sheetBackground,wire->pen().widthF()+jumpClearance,
                                                        Qt::SolidLine,Qt::RoundCap));
                casing->setData(displayRole,JumpCasing);casing->setZValue(-0.5);
                auto* arc=scene->addPath(jump,wire->pen());
                arc->setData(2,wire->data(2));arc->setData(3,wire->data(3));
                arc->setToolTip(wire->toolTip());arc->setZValue(-0.25);
            }
            return at+QPointF(other.x()<at.x() ? -jumpRadius : jumpRadius,0);
        };
        wire->setLine(QLineF(trim(line.p1(),line.p2()),trim(line.p2(),line.p1())));
    }
}

QJsonObject Sheet::layout(const Circuit& circuit) {
    QGraphicsScene scene;
    QMap<QString,NetRoute> routes;
    QSet<QPoint> crossings;
    drawSchematic(&scene,circuit,routes,crossings,{},{},{});
    auto bounds=scene.itemsBoundingRect().adjusted(-40,-40,40,40);
    // Keep room below the circuit so new parts can be dropped into empty grid cells.
    bounds.setHeight(std::max(bounds.height(),320.0));
    bounds.setWidth(std::max(bounds.width(),520.0));
    QJsonArray shapes,pins;
    QJsonObject centers;
    const auto positions=circuit.document["layout"].toObject();
    const auto parts=circuit.document["parts"].toObject();
    for (const auto& id:parts.keys()) {
        const auto at=position(positions[id].toObject())*pitch;
        centers[id]=QJsonArray{at.x(),at.y()};
    }
    auto color=[](const QBrush& brush) {
        return brush.style()==Qt::NoBrush ? QString("none") : brush.color().name();
    };
    for (auto* item:scene.items(Qt::AscendingOrder)) {
        const auto id=item->data(0).toString();
        const auto at=item->scenePos();
        if (const auto pin=item->data(1).toString(); !pin.isEmpty()) {
            pins.append(QJsonObject{{"id",pin},{"x",at.x()},{"y",at.y()}});
            continue;
        }
        if (auto* text=qgraphicsitem_cast<QGraphicsSimpleTextItem*>(item)) {
            const QFontMetricsF metrics(text->font());
            const QStringList lines=text->text().split(u'\n');
            for (qsizetype row=0;row<lines.size();++row)
                shapes.append(QJsonObject{{"text",lines[row]},{"id",id},{"x",at.x()},
                    {"y",at.y()+metrics.ascent()+row*metrics.lineSpacing()},
                    {"size",metrics.height()-metrics.leading()},{"bold",text->font().bold()},
                    {"fill",color(text->brush())}});
            continue;
        }
        QPainterPath path;
        QPen pen;
        QBrush fill(Qt::NoBrush);
        if (auto* wire=qgraphicsitem_cast<QGraphicsLineItem*>(item)) {
            path.moveTo(wire->line().p1());path.lineTo(wire->line().p2());pen=wire->pen();
        } else if (auto* symbol=qgraphicsitem_cast<QGraphicsPathItem*>(item)) {
            path=symbol->path();pen=symbol->pen();fill=symbol->brush();
        } else if (auto* dot=qgraphicsitem_cast<QGraphicsEllipseItem*>(item)) {
            path.addEllipse(dot->rect());pen=dot->pen();fill=dot->brush();
        } else continue;
        path=item->sceneTransform().map(path);
        QString data;
        for (int i=0;i<path.elementCount();++i) {
            const auto p=path.elementAt(i);
            if (p.isMoveTo()) data+="M";
            else if (p.isLineTo()) data+="L";
            else if (p.type==QPainterPath::CurveToElement) data+="C";
            data+=QString::number(p.x,'f',3)+" "+QString::number(p.y,'f',3)+" ";
        }
        shapes.append(QJsonObject{{"d",data},{"id",id},{"fill",color(fill)},
            {"stroke",pen.style()==Qt::NoPen ? QString("none") : pen.color().name()},
            {"width",pen.widthF()}});
    }
    return {{"viewBox",QJsonArray{bounds.x(),bounds.y(),bounds.width(),bounds.height()}},
        {"pitch",pitch},{"centers",centers},{"shapes",shapes},{"pins",pins}};
}

void Sheet::exportSchematic(const QString& filename,double widthMm,bool annotations) const {
    if(!std::isfinite(widthMm) || widthMm<1 || widthMm>1000)
        throw std::runtime_error("schematic width must be between 1 and 1000 mm");
    widthMm=qRound(widthMm*100)/100.0;
    const auto suffix=QFileInfo(filename).suffix().toLower();
    if(suffix!="svg" && suffix!="pdf") throw std::runtime_error("schematic output must be SVG or PDF");

    // Copy the live drawing in stacking order, including any routes retained during editing.
    // The view's camera, background and transient items do not participate.
    QGraphicsScene drawing;
    QList<QGraphicsPathItem*> paths,casing;
    QList<QGraphicsLineItem*> wires;
    QList<QGraphicsEllipseItem*> junctions;
    QList<ExportLabel*> labels;
    for(auto* item:scene()->items(Qt::AscendingOrder)) {
        const int role=item->data(displayRole).toInt();
        if(role==EditorOnly) continue;
        QGraphicsItem* copy=nullptr;
        if(const auto* shape=qgraphicsitem_cast<QGraphicsPathItem*>(item)) {
            auto pen=shape->pen();
            pen.setColor(role==JumpCasing ? Qt::white : Qt::black);
            pen.setWidthF(role==JumpCasing ? 2+jumpClearance : 2);
            auto* path=drawing.addPath(shape->path(),pen,
                shape->brush().style()==Qt::NoBrush ? QBrush(Qt::NoBrush) : QBrush(Qt::black));
            (role==JumpCasing ? casing : paths).append(path);
            path->setData(0,item->data(0));
            copy=path;
        } else if(const auto* wire=qgraphicsitem_cast<QGraphicsLineItem*>(item)) {
            auto pen=wire->pen();
            pen.setColor(Qt::black);pen.setWidthF(1.8);
            auto* line=drawing.addLine(wire->line(),pen);
            wires.append(line);
            copy=line;
        } else if(const auto* dot=qgraphicsitem_cast<QGraphicsEllipseItem*>(item)) {
            auto* junction=drawing.addEllipse(dot->rect(),Qt::NoPen,Qt::black);
            junctions.append(junction);
            copy=junction;
        } else if(const auto* text=qgraphicsitem_cast<QGraphicsSimpleTextItem*>(item)) {
            const auto kind=LabelRole(text->data(labelRole).toInt());
            if(!annotations && (kind==LabelRole::Section || kind==LabelRole::Title)) continue;
            auto* label=new ExportLabel(*text);
            drawing.addItem(label);
            labels.append(label);
            copy=label;
        }
        if(copy) {
            copy->setPos(item->pos());
            copy->setRotation(item->rotation());
            copy->setZValue(item->zValue());
        }
    }

    auto bounds=drawing.itemsBoundingRect();
    if(bounds.isEmpty()) bounds={0,0,32,32};
    const double marginMm=std::min(2.0,widthMm/8);
    const double unitsPerMm=bounds.width()/(widthMm-2*marginMm);
    const double margin=marginMm*unitsPerMm;
    const auto frame=bounds.adjusted(-margin,-margin,margin,margin);
    for(auto* path:paths) {
        auto pen=path->pen();
        pen.setWidthF(0.20*unitsPerMm);
        path->setPen(pen);
    }
    for(auto* path:casing) {
        auto pen=path->pen();
        // Keep a white rim in print while limiting the casing to the jump's crown.
        pen.setWidthF(std::min(0.50*unitsPerMm,2.0+jumpClearance));
        path->setPen(pen);
    }
    for(auto* wire:wires) {
        auto pen=wire->pen();
        pen.setWidthF(0.20*unitsPerMm);
        wire->setPen(pen);
    }
    for(auto* dot:junctions) {
        const double r=0.35*unitsPerMm;
        dot->setRect(-r,-r,2*r,2*r);
    }

    // Reserve the complete ink box at the final size before placing the next label.
    const double clearance=0.2*unitsPerMm;
    const double labelClearance=0.4*unitsPerMm;
    const QRectF horizontalArea=frame.adjusted(clearance,0,-clearance,0);
    QList<QRectF> placed;
    std::stable_sort(labels.begin(),labels.end(),[](const ExportLabel* a,const ExportLabel* b){
        return a->role<b->role;
    });
    QList<QPoint> offsets;
    constexpr double stepMm=0.25;
    constexpr int steps=32;
    for(int y=-steps;y<=steps;++y)
        for(int x=-steps;x<=steps;++x)
            if(x*x+y*y<=steps*steps) offsets.append({x,y});
    std::stable_sort(offsets.begin(),offsets.end(),[](QPoint a,QPoint b){
        return QPoint::dotProduct(a,a)<QPoint::dotProduct(b,b);
    });
    for(auto* label:labels) {
        // Cancel the scene-to-page scale so the role's point size is physical.
        label->setScale(publicationPoints(label->role)*25.4/72*unitsPerMm/label->font().pixelSize());
        const auto ink=label->inkBounds();
        if(ink.isEmpty()) continue;
        const QPointF toAnchor=label->anchor-ink.center();
        const auto padded=ink.translated(toAnchor).adjusted(-clearance,-clearance,clearance,clearance);
        const QPointF outward=label->anchor-label->origin;
        const double reach=steps*stepMm*unitsPerMm;
        const auto nearby=drawing.items(padded.adjusted(-reach,-reach,reach,reach),Qt::IntersectsItemBoundingRect);
        QList<QPainterPath> obstacles;
        for(auto* item:nearby) {
            if(qgraphicsitem_cast<ExportLabel*>(item)) continue;
            QPainterPath obstacle;
            if(const auto* path=qgraphicsitem_cast<QGraphicsPathItem*>(item)) {
                const auto part=path->data(0).toString();
                if(!part.isEmpty() && !(part==label->owner && label->role==LabelRole::Meter)) {
                    obstacle.addRect(path->sceneBoundingRect());
                } else {
                    QPainterPathStroker stroke(path->pen());
                    obstacle=path->mapToScene(stroke.createStroke(path->path()));
                    if(path->brush().style()!=Qt::NoBrush) obstacle.addPath(path->mapToScene(path->path()));
                }
            } else {
                obstacle=item->mapToScene(item->shape());
            }
            obstacles.append(obstacle);
        }
        bool found=false;
        for(auto offset:offsets) {
            const QPointF delta=QPointF(offset)*(stepMm*unitsPerMm);
            if(label->role==LabelRole::Meter && !offset.isNull()) break;
            if(!label->owner.isEmpty() && QPointF::dotProduct(outward,outward+delta)<0) continue;
            const auto box=padded.translated(delta);
            if(box.left()<horizontalArea.left() || box.right()>horizontalArea.right()) continue;
            const auto labelBox=ink.translated(toAnchor+delta)
                .adjusted(-labelClearance,-labelClearance,labelClearance,labelClearance);
            if(std::any_of(placed.cbegin(),placed.cend(),[&](const QRectF& other){return labelBox.intersects(other);}) ||
               std::any_of(obstacles.cbegin(),obstacles.cend(),[&](const QPainterPath& other){return other.intersects(box);}))
                continue;
            label->moveBy(toAnchor.x()+delta.x(),toAnchor.y()+delta.y());
            placed.append(labelBox);
            found=true;
            break;
        }
        if(!found)
            throw std::runtime_error(QString("Cannot place label %1 (%2) at %3 mm near grid (%4, %5)")
                .arg(label->text(),label->owner).arg(widthMm)
                .arg(label->anchor.x()/pitch,0,'f',1).arg(label->anchor.y()/pitch,0,'f',1).toStdString());
    }

    // Very narrow or unusual sheets can grow past the nominal margin after
    // stroke correction. Expand vertically and keep the requested page width.
    const auto actual=drawing.itemsBoundingRect();
    const double top=std::min(frame.top(),actual.top()-margin/2);
    const double bottom=std::max(frame.bottom(),actual.bottom()+margin/2);
    const QRectF source(frame.left(),top,frame.width(),bottom-top);
    if(!actual.isEmpty() && (actual.left()<source.left() || actual.right()>source.right()))
        throw std::runtime_error("schematic strokes do not fit the requested width");
    constexpr int dpi=2540; // 100 device units per millimetre.
    const double height=widthMm*100*source.height()/source.width();
    if(!std::isfinite(height) || height>std::numeric_limits<int>::max())
        throw std::runtime_error("schematic page is too tall");
    const QSize page(qRound(widthMm*100),qCeil(height));
    QSaveFile output(filename);
    if(!output.open(QIODevice::WriteOnly)) throw std::runtime_error(output.errorString().toStdString());
    auto render=[&](QPaintDevice& device) {
        QPainter painter;
        if(!painter.begin(&device)) throw std::runtime_error("cannot start schematic export");
        painter.setRenderHint(QPainter::Antialiasing);
        const QRectF target(0,0,device.width(),device.height());
        painter.fillRect(target,Qt::white);
        // Render at the requested scale even if Qt rounds the PDF page box.
        drawing.render(&painter,QRectF(0,0,page.width(),height),source,Qt::IgnoreAspectRatio);
        if(!painter.end()) throw std::runtime_error("cannot finish schematic export");
    };
    if(suffix=="svg") {
        QSvgGenerator svg;
        svg.setOutputDevice(&output);
        svg.setResolution(dpi);
        svg.setSize(page);
        svg.setViewBox(QRect(QPoint(),page));
        render(svg);
    } else {
        // Outline the same glyphs so the PDF preserves variable font weights.
        for(auto* label:labels) label->useOutline=true;
        QPdfWriter pdf(&output);
        pdf.setResolution(dpi);
        if(!pdf.setPageSize(QPageSize(QSizeF(widthMm,page.height()/100.0),QPageSize::Millimeter,
                                     QString(),QPageSize::ExactMatch)) ||
           !pdf.setPageMargins(QMarginsF(),QPageLayout::Millimeter))
            throw std::runtime_error("cannot set schematic PDF page size");
        render(pdf);
    }
    if(!output.commit()) throw std::runtime_error(output.errorString().toStdString());
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
    p->fillRect(rect,sheetBackground);p->setPen(QPen(QColor("#d7e1df"),1.3));
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
