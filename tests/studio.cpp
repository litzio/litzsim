#include "studio.h"
#include "waves.h"
#include "solver.h"
#include <QEventLoop>
#include <QTimer>
#include <QJsonDocument>
#include <QApplication>
#include <QDir>
#include <QTemporaryDir>
#include <QGraphicsItem>
#include <QMouseEvent>
#include <QRegularExpression>
#include <QSvgRenderer>
#include <QXmlStreamReader>
#include <algorithm>
#include <cmath>
#include <cstdio>

namespace {
QPointF terminal(const Sheet& sheet,const QString& name) {
    for(auto* item:sheet.scene()->items())
        if(item->data(1).toString()==name) return item->scenePos();
    throw std::runtime_error(("missing scene terminal: "+name).toStdString());
}

int partLabelSize(const Sheet& sheet,const QString& id,bool value=false) {
    for(auto* item:sheet.scene()->items()) {
        const auto* label=qgraphicsitem_cast<QGraphicsSimpleTextItem*>(item);
        if(label && label->data(0).toString()==id && (label->text()!=id)==value)
            return label->font().pixelSize();
    }
    throw std::runtime_error(("missing part label: "+id).toStdString());
}

void verifyMark(const Sheet& sheet,const QString& id,const QString& text,const QString& pin,
                int minSize=22,int maxSize=22) {
    int matches=0;
    const auto target=terminal(sheet,id+"."+pin);
    for(auto* item:sheet.scene()->items()) {
        const auto* label=qgraphicsitem_cast<QGraphicsSimpleTextItem*>(item);
        if(!label || label->data(0).toString()!=id || label->text()!=text) continue;
        const int size=label->font().pixelSize();
        if(minSize>0 && (size<minSize || size>maxSize))
            throw std::runtime_error(("terminal mark outside size range: "+id+" "+text).toStdString());
        const auto area=label->sceneBoundingRect();
        const auto nearest=QPointF(std::clamp(target.x(),area.left(),area.right()),
                                   std::clamp(target.y(),area.top(),area.bottom()));
        // Single glyphs are anchored within one grid step. Signal names grow outward from their anchor.
        const bool near=minSize>0 ? QLineF(area.center(),target).length()<32
                                  : QLineF(nearest,target).length()<18;
        if(near && label->brush().color()==Qt::black) ++matches;
    }
    if(matches!=1) throw std::runtime_error(("missing or misplaced mark: "+id+" "+text).toStdString());
}

void verifySymbol(const Sheet& sheet,const QString& id,const QStringList& pins,bool current=false) {
    for(auto* item:sheet.scene()->items()) {
        const auto* symbol=qgraphicsitem_cast<QGraphicsPathItem*>(item);
        if(!symbol || symbol->data(0).toString()!=id) continue;
        const auto path=symbol->path();
        QList<QPointF> ends;
        QList<QLineF> strokes;
        QLineF shaft;
        for(int i=0;i<path.elementCount();++i) {
            const auto point=path.elementAt(i);
            ends.append(symbol->mapToScene(point.x,point.y));
            if(!current || !point.isLineTo() || i==0) continue;
            const auto start=path.elementAt(i-1);
            const QLineF stroke(ends[i-1],ends.last());
            strokes.append(stroke);
            if(start.x*point.x<0 && start.y==point.y && stroke.length()>15) shaft=stroke;
        }
        bool connected=true;
        for(const auto& pin:pins)
            if(!ends.contains(terminal(sheet,id+"."+pin))) connected=false;
        if(!connected || symbol->pen().color()!=Qt::black) continue;
        if(!current) return;
        const auto direction=terminal(sheet,id+".n")-terminal(sheet,id+".p");
        if(QPointF::dotProduct(direction,shaft.p2()-shaft.p1())<0)
            shaft=QLineF(shaft.p2(),shaft.p1());
        // Two angled strokes must meet at the tip on the n side of the shaft.
        const auto wings=std::count_if(strokes.begin(),strokes.end(),[&](QLineF stroke){
            if(stroke.p2()==shaft.p2()) stroke=QLineF(stroke.p2(),stroke.p1());
            const auto wing=stroke.p2()-stroke.p1();
            return stroke.p1()==shaft.p2() && QPointF::dotProduct(direction,wing)<0 &&
                   (direction.x()*wing.y()-direction.y()*wing.x())!=0;
        });
        if(wings==2) return;
    }
    throw std::runtime_error(("symbol disagrees with its terminals: "+id).toStdString());
}

void verifySchematic(const QString& examples) {
    for(const QString name:{"rc","buck","pfc"}) {
        const auto path=examples+"/"+name+".litzsim";
        Sheet sheet(path,Circuit::load(path),nullptr);
        const QString probe=name=="pfc" ? "Vdc" : "VC";
        verifyMark(sheet,probe,"+","p");
        QSet<QPoint> jumps;
        for(auto* item:sheet.scene()->items()) {
            const auto* wire=qgraphicsitem_cast<QGraphicsLineItem*>(item);
            const auto* junction=qgraphicsitem_cast<QGraphicsEllipseItem*>(item);
            const auto* text=qgraphicsitem_cast<QGraphicsSimpleTextItem*>(item);
            const auto* symbol=qgraphicsitem_cast<QGraphicsPathItem*>(item);
            if((wire && wire->pen().color()!=Qt::black) ||
               (junction && junction->brush().color()!=Qt::black) ||
               (symbol && symbol->pen().color()!=Qt::black &&
                !(symbol->zValue()==-0.5 && symbol->pen().color()==QColor("#fbfcfa"))) ||
               (text && text->brush().color()!=Qt::black))
                throw std::runtime_error(("unexpected diagram color or routing warning: "+name).toStdString());
            const auto net=item->data(3).toString();
            if(symbol && !net.isEmpty()) {
                const auto rect=symbol->path().boundingRect();
                jumps.insert(QPointF(rect.center().x(),rect.bottom()).toPoint()/32);
                const QPointF center(rect.center().x(),rect.bottom());
                QSet<qreal> verticalEnds,horizontalEnds;
                for(auto* candidate:sheet.scene()->items()) {
                    const auto* line=qgraphicsitem_cast<QGraphicsLineItem*>(candidate);
                    if(!line) continue;
                    const auto segment=line->line();
                    if(segment.p1()==center && segment.dx()==0) verticalEnds.insert(segment.y2());
                    if(segment.p2()==center && segment.dx()==0) verticalEnds.insert(segment.y1());
                    if(candidate->data(3).toString()!=net || segment.dy()!=0) continue;
                    if(segment.p1()==QPointF(rect.left(),center.y())) horizontalEnds.insert(segment.x2());
                    if(segment.p2()==QPointF(rect.left(),center.y())) horizontalEnds.insert(segment.x1());
                    if(segment.p1()==QPointF(rect.right(),center.y())) horizontalEnds.insert(segment.x2());
                    if(segment.p2()==QPointF(rect.right(),center.y())) horizontalEnds.insert(segment.x1());
                    if(segment.y1()==center.y() && std::min(segment.x1(),segment.x2())<rect.right() &&
                       std::max(segment.x1(),segment.x2())>rect.left())
                        throw std::runtime_error("straight horizontal stroke under jump");
                }
                if(verticalEnds!=QSet<qreal>{center.y()-32,center.y()+32} ||
                   horizontalEnds!=QSet<qreal>{center.x()-32,center.x()+32})
                    throw std::runtime_error("jump does not preserve the crossing strokes");
            }
            const bool straightRail=(name=="rc" && (net=="input" || net=="output")) ||
                                    (name=="buck" && (net=="output" || net=="ground" || net=="gate"));
            if(wire && straightRail && wire->line().dy()!=0)
                throw std::runtime_error(("unnecessary bend on "+name+" "+net).toStdString());
        }
        const QSet<QPoint> expected=name=="pfc" ? QSet<QPoint>{{15,7},{15,10},{20,10}} : QSet<QPoint>{};
        if(jumps!=expected) throw std::runtime_error(("unexpected wire crossings: "+name).toStdString());
        const auto gates=sheet.circuit.document["control"].toObject()["gateSignals"].toObject();
        QStringList transistors;
        for(auto it=gates.begin();it!=gates.end();++it) {
            const auto pin=it.value().toString();
            transistors.append(pin.section('.',0,0));
            const auto end=terminal(sheet,pin);
            for(auto* item:sheet.scene()->items()) {
                const auto* wire=qgraphicsitem_cast<QGraphicsLineItem*>(item);
                if(wire && (wire->line().p1()==end || wire->line().p2()==end))
                    throw std::runtime_error("controller mapping drawn as a physical wire");
            }
        }
        if(name=="buck") transistors.append("Q1");
        const QStringList sensors=name=="pfc" ? QStringList{"IL1","IL2"} : QStringList{};
        for(int turn=0;turn<4;++turn) {
            verifySymbol(sheet,probe,{"p","n"});
            verifyMark(sheet,probe,"+","p");
            verifyMark(sheet,probe,"−","n");
            for(const auto& id:sensors) verifySymbol(sheet,id,{"p","n"},true);
            for(const auto& id:transistors) {
                verifySymbol(sheet,id,{"d","g","s"});
                const int valueSize=partLabelSize(sheet,"L1",true);
                const int idSize=partLabelSize(sheet,id);
                verifyMark(sheet,id,"D","d",valueSize,idSize);
                verifyMark(sheet,id,"G","g",valueSize,idSize);
                verifyMark(sheet,id,"S","s",valueSize,idSize);
            }
            for(auto it=gates.begin();it!=gates.end();++it)
                verifyMark(sheet,it.value().toString().section('.',0,0),it.key(),"g",0);
            sheet.circuit.rotate(probe);
            for(const auto& id:sensors+transistors) sheet.circuit.rotate(id);
            sheet.redraw();
        }
    }
}

QByteArray readExport(const QString& path) {
    QFile file(path);
    if(!file.open(QIODevice::ReadOnly)) throw std::runtime_error("missing schematic export");
    return file.readAll();
}

void verifySvg(const QByteArray& bytes,double widthMm) {
    QXmlStreamReader xml(bytes);
    bool root=false,path=false,text=false;
    while(!xml.atEnd()) {
        if(xml.readNext()!=QXmlStreamReader::StartElement) continue;
        if(xml.name()==u"svg") {
            const auto width=xml.attributes().value("width").toString();
            const auto box=xml.attributes().value("viewBox").toString().split(' ');
            root=width.endsWith("mm") && std::abs(width.chopped(2).toDouble()-widthMm)<0.01 &&
                 box.size()==4 && box[2].toInt()==qRound(widthMm*100) && box[3].toInt()>0;
        }
        if(xml.name()==u"path") path=true;
        if(xml.name()==u"text") text=true;
        if(xml.name()==u"image") throw std::runtime_error("bitmap embedded in SVG");
    }
    if(xml.hasError() || !root || !path || !text)
        throw std::runtime_error("SVG is missing its physical width, viewBox or vector drawing");
}

void verifyPrintLabels(const QByteArray& bytes,const Circuit& circuit,bool annotations) {
    const auto parts=circuit.document["parts"].toObject();
    const auto gates=circuit.document["control"].toObject()["gateSignals"].toObject();
    const auto title=circuit.document["title"].toString();
    QSet<QString> sections,seen;
    for(auto entry:circuit.document["annotations"].toArray())
        sections.insert(entry.toObject()["text"].toString());

    QXmlStreamReader svg(bytes);
    QTransform transform;
    QSizeF sheetSize;
    bool isSymbol=false;
    qreal stroke=0;
    QString label;
    QRectF labelBox;
    QList<QPair<double,int>> styles;
    QList<QPair<QString,QRectF>> boxes;
    QList<QRectF> symbols;
    while(!svg.atEnd()) {
        const auto token=svg.readNext();
        const auto attrs=svg.attributes();
        if(token==QXmlStreamReader::StartElement && svg.name()==u"svg") {
            const auto box=attrs.value("viewBox").toString().split(' ');
            sheetSize={box.value(2).toDouble(),box.value(3).toDouble()};
        } else if(token==QXmlStreamReader::StartElement && svg.name()==u"g") {
            const auto matrix=attrs.value("transform").toString();
            const auto m=matrix.mid(7,matrix.size()-8).split(',');
            if(m.size()==6) transform={m[0].toDouble(),m[1].toDouble(),m[2].toDouble(),
                                       m[3].toDouble(),m[4].toDouble(),m[5].toDouble()};
            isSymbol=attrs.value("stroke-linejoin")==u"round";
            stroke=attrs.value("stroke-width").toDouble()/2;
        } else if(token==QXmlStreamReader::StartElement && svg.name()==u"path" && isSymbol) {
            const auto d=attrs.value("d").toUtf8();
            QSvgRenderer shape(QByteArray("<svg xmlns=\"http://www.w3.org/2000/svg\"><path id=\"symbol\" d=\"")+
                               d+"\"/></svg>");
            const auto local=shape.boundsOnElement("symbol").adjusted(-stroke,-stroke,stroke,stroke);
            symbols.append(transform.mapRect(local));
        } else if(token==QXmlStreamReader::StartElement && svg.name()==u"text") {
            const int size=attrs.value("font-size").toInt(),weight=attrs.value("font-weight").toInt();
            QFont font(attrs.value("font-family").toString());
            font.setPixelSize(size);
            font.setWeight(QFont::Weight(weight));
            const QPointF baseline(attrs.value("x").toDouble(),attrs.value("y").toDouble());
            const auto run=svg.readElementText();
            QPainterPath ink;
            ink.addText(baseline,font,run);
            label+=run;
            labelBox=labelBox.united(transform.mapRect(ink.boundingRect()));
            styles.append({size*std::hypot(transform.m11(),transform.m12())*72/2540,weight});
        } else if(token==QXmlStreamReader::EndElement && svg.name()==u"g" && !styles.isEmpty()) {
            const bool heading=label==title || sections.contains(label);
            const bool bold=heading || parts.contains(label) || label=="+" || label=="−";
            const double points=label==title ? 9.0*25/16 : bold ? 9 : 8;
            if(heading && !annotations) throw std::runtime_error("unexpected title or section in default export");
            for(auto [measured,weight]:styles) {
                if(std::abs(measured-points)>0.01 || weight!=(bold ? 700 : 400))
                    throw std::runtime_error(("incorrect physical font size or weight: "+label).toStdString());
            }
            if(!QRectF(QPointF(),sheetSize).contains(labelBox)) throw std::runtime_error("clipped SVG label");
            seen.insert(label);
            boxes.append({label,labelBox});
            label.clear();
            labelBox={};
            styles.clear();
        }
    }
    if(svg.hasError() || symbols.size()!=parts.size()) throw std::runtime_error("incorrect SVG symbol count");
    for(auto part=parts.begin();part!=parts.end();++part)
        if(!seen.contains(part.key())) throw std::runtime_error("missing component ID in SVG");
    for(auto gate=gates.begin();gate!=gates.end();++gate)
        if(!seen.contains(gate.key()) || seen.contains("제어 "+gate.key()))
            throw std::runtime_error("incorrect gate signal label in SVG");
    if(annotations && ((!title.isEmpty() && !seen.contains(title)) || !seen.contains(sections)))
        throw std::runtime_error("missing title or section in annotated export");

    for(qsizetype i=0;i<boxes.size();++i) {
        const auto& [label,box]=boxes[i];
        for(qsizetype j=0;j<i;++j)
            if(box.intersects(boxes[j].second))
                throw std::runtime_error((label+" overlaps "+boxes[j].first+" in SVG").toStdString());
        for(const auto& symbol:symbols) {
            // A and V are contained by their own instrument symbols.
            if(symbol.contains(box) && (label=="A" || label=="V")) continue;
            if(symbol.intersects(box)) throw std::runtime_error((label+" overlaps a symbol in SVG").toStdString());
        }
    }
}

QString verifyPdf(const QByteArray& bytes,double widthMm) {
    auto pdf=QString::fromLatin1(bytes);
    const auto page=QRegularExpression(R"(/MediaBox\s*\[0\s+0\s+([\d.]+)\s+([\d.]+)\])").match(pdf);
    if(!bytes.startsWith("%PDF-") || !page.hasMatch() || page.captured(2).toDouble()<=0 ||
       std::abs(page.captured(1).toDouble()*25.4/72-widthMm)>0.18 ||
       pdf.contains(QRegularExpression(R"(/Subtype\s*/Image)")))
        throw std::runtime_error("PDF is missing its page size or contains a bitmap");
    // Qt writes timestamps and a fresh document ID on each export.
    pdf.replace(QRegularExpression(R"(/(?:CreationDate|ModDate)\s*\([^\r\n]*\))"),QString());
    pdf.replace(QRegularExpression(R"(xmp:(?:CreateDate|ModifyDate|MetadataDate)="[^"]*")"),QString());
    pdf.replace(QRegularExpression(R"(xmpMM:(?:DocumentID|InstanceID)="[^"]*")"),QString());
    pdf.replace(QRegularExpression(R"(/ID\s*\[\s*<[0-9a-fA-F]+>\s*<[0-9a-fA-F]+>\s*\])"),QString());
    return pdf;
}

void verifyPdfOutlines(const QByteArray& bytes) {
    const QRegularExpression contentRef(R"(/Contents\s+(\d+)\s+0\s+R)");
    const auto ref=contentRef.match(QString::fromLatin1(bytes));
    if(!ref.hasMatch()) throw std::runtime_error("missing PDF page content");
    const auto object=bytes.indexOf("\n"+ref.captured(1).toLatin1()+" 0 obj");
    const auto stream=bytes.indexOf("\nstream\n",object)+8;
    const auto end=bytes.indexOf("\nendstream",stream);
    if(object<0 || stream<8 || end<stream) throw std::runtime_error("missing PDF content stream");
    auto drawing=bytes.mid(stream,end-stream);
    if(bytes.mid(object,stream-object).contains("/FlateDecode")) {
        // qUncompress expects a four-byte big-endian size hint before the zlib stream.
        drawing=qUncompress(QByteArray("\0\1\0\0",4)+drawing);
    }
    if(bytes.contains("/FontFile") || !drawing.contains(" c\n") || !drawing.contains("\nf\n"))
        throw std::runtime_error("PDF title was not written as vector glyph outlines");
}

void verifyExports(Studio& studio,const QString& examples,const QString& directory) {
    const auto input=examples+"/pfc.litzsim";
    auto* sheet=studio.open(input);
    const auto document=sheet->circuit.document;
    const auto before=directory+"/before",after=directory+"/after",pending=directory+"/pending";
    for(const auto& ext:{".svg",".pdf"}) sheet->exportSchematic(before+ext,182);
    studio.show();
    QApplication::processEvents();
    auto centerOnDrawing=[&] {
        const auto bounds=sheet->scene()->itemsBoundingRect().adjusted(-35,-35,35,35);
        sheet->fitInView(bounds,Qt::KeepAspectRatio);
        sheet->centerOn(bounds.center());
    };
    auto click=[&](QPointF at) {
        const QPoint point=sheet->mapFromScene(at),global=sheet->viewport()->mapToGlobal(point);
        QMouseEvent press(QEvent::MouseButtonPress,point,global,Qt::LeftButton,Qt::LeftButton,Qt::NoModifier);
        QMouseEvent release(QEvent::MouseButtonRelease,point,global,Qt::LeftButton,Qt::NoButton,Qt::NoModifier);
        QApplication::sendEvent(sheet->viewport(),&press);
        QApplication::sendEvent(sheet->viewport(),&release);
    };
    centerOnDrawing();
    for(auto* item:sheet->scene()->items()) {
        const auto* label=qgraphicsitem_cast<QGraphicsSimpleTextItem*>(item);
        if(label && label->text()=="Q1H") {
            click(label->sceneBoundingRect().center());
            break;
        }
    }
    bool selected=false;
    for(auto* item:sheet->scene()->items()) {
        const auto* path=qgraphicsitem_cast<QGraphicsPathItem*>(item);
        if(path && path->pen().color()==QColor("#007d72")) selected=true;
    }
    if(!selected) throw std::runtime_error("test could not select a schematic symbol");
    sheet->scale(1.2,1.2);
    for(const auto& ext:{".svg",".pdf"}) sheet->exportSchematic(after+ext,182);
    centerOnDrawing();
    click(terminal(*sheet,"Q1H.g"));
    int visibleHandles=0;
    for(auto* item:sheet->scene()->items()) {
        const auto* rect=qgraphicsitem_cast<QGraphicsRectItem*>(item);
        if(rect && rect->pen().style()!=Qt::NoPen) ++visibleHandles;
    }
    if(visibleHandles<=3) throw std::runtime_error("test did not start a wire");
    sheet->exportSchematic(pending+".svg",182);

    const auto svg=readExport(before+".svg");
    verifySvg(svg,182);
    verifyPrintLabels(svg,sheet->circuit,false);
    if(svg!=readExport(after+".svg")) throw std::runtime_error("selection or zoom changed SVG");
    if(svg!=readExport(pending+".svg")) throw std::runtime_error("pending wire changed SVG");
    const auto pdf=verifyPdf(readExport(before+".pdf"),182);
    const auto selectedPdf=verifyPdf(readExport(after+".pdf"),182);
    if(pdf!=selectedPdf) throw std::runtime_error("selection or zoom changed PDF");
    if(sheet->circuit.document!=document || !sheet->history.isClean())
        throw std::runtime_error("export modified the circuit");
    studio.hide();

    // A title-only page isolates the glyph paths from the schematic paths.
    Sheet title("",{QJsonObject{{"title","QΩ한"}}},nullptr);
    const auto titlePdf=directory+"/title.pdf";
    title.exportSchematic(titlePdf,88.9,true);
    verifyPdfOutlines(readExport(titlePdf));

    const auto cli=QDir(QCoreApplication::applicationDirPath()).filePath("litzsim-cli");
    for(const auto& ext:{".svg",".pdf"}) {
        const auto output=directory+"/rc"+ext;
        if(QProcess::execute(cli,{"export",examples+"/rc.litzsim",output,"--width-mm","88.9"})!=0)
            throw std::runtime_error("CLI schematic export failed");
        const auto bytes=readExport(output);
        if(QString(ext)==".svg") {
            verifySvg(bytes,88.9);
            verifyPrintLabels(bytes,Circuit::load(examples+"/rc.litzsim"),false);
        }
        else verifyPdf(bytes,88.9);
    }
    const auto annotated=directory+"/annotated.svg";
    if(QProcess::execute(cli,{"export",input,annotated,"--width-mm","182","--annotations"})!=0)
        throw std::runtime_error("CLI annotated schematic export failed");
    verifyPrintLabels(readExport(annotated),{document},true);

    const auto buck=examples+"/buck.litzsim";
    Sheet buckSheet(buck,Circuit::load(buck),nullptr);
    for(double width:{88.9,182.0}) {
        const auto output=directory+"/buck.svg";
        buckSheet.exportSchematic(output,width);
        const auto bytes=readExport(output);
        verifySvg(bytes,width);
        verifyPrintLabels(bytes,buckSheet.circuit,false);
    }
}
}

int main(int argc,char** argv) {
    QApplication app(argc,argv);
    if (argc!=2) return 2;
    QTemporaryDir dir;
    Studio studio(dir.path());
    QString examples=QString::fromLocal8Bit(argv[1]);
    try {
        verifySchematic(examples);
        verifyExports(studio,examples,dir.path());
    } catch(const std::exception& error) {
        std::fprintf(stderr,"%s\n",error.what());
        return 8;
    }
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
    QEventLoop wait;
    QString output;
    studio.showResult=[&](const QString& path){output=path;wait.quit();};
    studio.startRun();
    auto snapshot=first->circuit;
    first->edit([](Circuit& c){c.move("C1",-7,19);});
    QTimer::singleShot(5000,&wait,&QEventLoop::quit);
    wait.exec();
    if (output.isEmpty() || Circuit::load(studio.runDirectory+"/circuit.litzsim").document!=snapshot.document) return 5;
    Waves result(output);
    qint64 row=0;
    simulate(snapshot,[&](double t,const QMap<QString,double>& values){
        if (result.at(row,0)!=t || result.at(row,1)!=values["VC_V"]) throw std::runtime_error("GUI/CLI mismatch");
        ++row;
    });
    if (row!=result.rows) return 6;
    first->edit([](Circuit& c){
        auto cfg=c.document["simulation"].toObject();cfg["stopTime_s"]=1000;c.document["simulation"]=cfg;
    });
    QEventLoop cancelWait;
    auto* runner=studio.findChild<QProcess*>();
    QObject::connect(runner,&QProcess::started,&studio,&Studio::stopRun);
    QObject::connect(runner,&QProcess::finished,&cancelWait,&QEventLoop::quit);
    studio.startRun();
    QTimer::singleShot(5000,&cancelWait,&QEventLoop::quit);cancelWait.exec();
    QFile report(studio.runDirectory+"/status.json");
    if(!report.open(QIODevice::ReadOnly) || QJsonDocument::fromJson(report.readAll()).object()["state"]!="canceled") return 7;
    return first->history.isClean() ? 1 : 0;
}
