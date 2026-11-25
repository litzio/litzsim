#include "solver.h"
#include <QJsonArray>
#include <QSaveFile>
#include <QSet>
#include <QTextStream>
#include <algorithm>
#include <cmath>
#include <stdexcept>
#include <vector>

namespace {
using Vector = std::vector<double>;
struct System {
    std::vector<Vector> a;
    Vector b;
    explicit System(int n) : a(n,Vector(n)), b(n) {}
    void add(int row,int col,double v) { if (row >= 0 && col >= 0) a[row][col] += v; }
    void rhs(int row,double v) { if (row >= 0) b[row] += v; }
    Vector solve() {
        const int n = static_cast<int>(b.size());
        for (int k=0;k<n;++k) {
            int pivot=k;
            for (int i=k+1;i<n;++i)
                if (std::abs(a[i][k]) > std::abs(a[pivot][k])) pivot=i;
            if (a[pivot][k]==0) throw std::runtime_error("singular circuit system");
            std::swap(a[pivot],a[k]); std::swap(b[pivot],b[k]);
            for (int i=k+1;i<n;++i) {
                double factor=a[i][k]/a[k][k];
                for (int j=k+1;j<n;++j) a[i][j]-=factor*a[k][j];
                b[i]-=factor*b[k];
            }
        }
        for (int i=n-1;i>=0;--i) {
            for (int j=i+1;j<n;++j) b[i]-=a[i][j]*b[j];
            b[i]/=a[i][i];
            if (!std::isfinite(b[i])) throw std::runtime_error("non-finite circuit solution");
        }
        return b;
    }
};
struct Device {
    QString id, type;
    QJsonObject params;
    int p,n,branch;
    double history;
};
double node(const Vector& x,int k) { return k<0 ? 0 : x.at(k); }
}
void simulate(const Circuit& circuit,const Sample& emitSample) {
    circuit.validate();
    auto missing=circuit.missingPins();
    if (!missing.isEmpty()) throw std::runtime_error(("unconnected: "+missing.join(", ")).toStdString());
    const auto nets=circuit.document["nets"].toObject();
    const auto parts=circuit.document["parts"].toObject();
    QMap<QString,QString> pinNet;
    for (auto net=nets.begin(); net!=nets.end(); ++net)
        for (const auto pin:net.value().toArray()) pinNet[pin.toString()]=net.key();
    QSet<QString> grounds;
    for (auto it=parts.begin();it!=parts.end();++it)
        if (it.value().toObject()["type"]=="ground") grounds.insert(pinNet.value(it.key()+".ref"));
    if (grounds.isEmpty()) throw std::runtime_error("missing reference ground");
    int size=0;
    QMap<QString,int> index;
    for (const auto& name:nets.keys()) index[name]=grounds.contains(name) ? -1 : size++;
    std::vector<Device> devices;
    for (auto it=parts.begin();it!=parts.end();++it) {
        auto part=it.value().toObject();
        QString type=part["type"].toString();
        if (type=="ground") continue;
        const auto pins=library().value(type).pins;
        Device d{it.key(),type,part["parameters"].toObject(),index[pinNet[it.key()+"."+pins[0]]],
                 index[pinNet[it.key()+"."+pins[1]]],-1,0};
        if (type=="capacitor" || type=="inductor" || type=="dc" || type=="sine" || type=="ammeter") d.branch=size++;
        else if (type!="resistor" && type!="voltmeter")
            throw std::runtime_error(("unsupported device: "+d.id).toStdString());
        for (const auto key:{"capacitance_F","inductance_H","resistance_ohm"})
            if (d.params.contains(key) && d.params[key].toDouble()<=0)
                throw std::runtime_error(("non-positive passive value: "+d.id).toStdString());
        d.history=d.params[type=="capacitor" ? "initialVoltage_V" : "initialCurrent_A"].toDouble();
        devices.push_back(d);
    }
    const auto cfg=circuit.document["simulation"].toObject();
    const double step=cfg["timeStep_s"].toDouble(), stop=cfg["stopTime_s"].toDouble();
    double t=0,h=0;
    for (;;) {
        System sys(size);
        for (const auto& d:devices) {
            if (d.type=="voltmeter") continue;
            if (d.type=="resistor") {
                const double g=1/d.params["resistance_ohm"].toDouble();
                sys.add(d.p,d.p,g); sys.add(d.n,d.n,g);
                sys.add(d.p,d.n,-g); sys.add(d.n,d.p,-g);
                continue;
            }
            const int k=d.branch;
            sys.add(d.p,k,1); sys.add(d.n,k,-1);
            if (d.type=="inductor" && h==0) {
                sys.add(k,k,1); sys.rhs(k,d.history);
                continue;
            }
            sys.add(k,d.p,1); sys.add(k,d.n,-1);
            if (d.type=="capacitor") {
                sys.add(k,k,-h/d.params["capacitance_F"].toDouble()); sys.rhs(k,d.history);
            } else if (d.type=="inductor") {
                const double z=d.params["inductance_H"].toDouble()/h;
                sys.add(k,k,-z); sys.rhs(k,-z*d.history);
            } else if (d.type=="dc") sys.rhs(k,d.params["voltage_V"].toDouble());
            else if (d.type=="sine") sys.rhs(k,d.params["amplitude_V"].toDouble()*
                std::sin(6.283185307179586*t*d.params["frequency_Hz"].toDouble()));
        }
        auto x=sys.solve();
        QMap<QString,double> values;
        for (auto& d:devices) {
            const double v=node(x,d.p)-node(x,d.n);
            if (d.type=="voltmeter") values[d.id+"_V"]=v;
            if (d.type=="ammeter") values[d.id+"_A"]=x[d.branch];
            if (d.type=="capacitor") d.history=v;
            if (d.type=="inductor") d.history=x[d.branch];
        }
        emitSample(t,values);
        if (t==stop) break;
        const double next=std::min(stop,t+step);
        h=next-t;
        if (h<=0) throw std::runtime_error("time step below clock resolution");
        t=next;
    }
}
void simulateCsv(const Circuit& circuit,const QString& filename) {
    QSaveFile file(filename);
    if (!file.open(QIODevice::WriteOnly)) throw std::runtime_error(file.errorString().toStdString());
    QTextStream out(&file);
    out.setRealNumberPrecision(17);
    simulate(circuit,[&](double time,const QMap<QString,double>& values){
        if (time==0) {
            out<<"time_s";
            for (const auto& name:values.keys()) out<<','<<name;
            out<<'\n';
        }
        out<<time;
        for (double value:values) out<<','<<value;
        out<<'\n';
    });
    out.flush();
    if (out.status()!=QTextStream::Ok || !file.commit()) throw std::runtime_error("result write failed");
}
