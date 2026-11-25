#include "solver.h"
#include "waves.h"
#include "pfc.h"
#include <memory>
#include <QJsonArray>
#include <QSaveFile>
#include <QSet>
#include <QTextStream>
#include <algorithm>
#include <cmath>
#include <limits>
#include <stdexcept>
#include <vector>

namespace {
using Vector = std::vector<double>;
struct SingularSystem : std::runtime_error {
    SingularSystem() : std::runtime_error("singular circuit system") {}
};
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
            if (a[pivot][k]==0) throw SingularSystem();
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
    int gateDriver=-1;
    bool conducting=false;
    bool driven=false;
};
double node(const Vector& x,int k) { return k<0 ? 0 : x.at(k); }
bool pulseHigh(const Device& pulse,double t) {
    const double cycles=t*pulse.params["frequency_Hz"].toDouble();
    return cycles-std::floor(cycles)<pulse.params["duty"].toDouble();
}
double nextPulseEdge(const Device& pulse,double t) {
    const double f=pulse.params["frequency_Hz"].toDouble();
    const double duty=pulse.params["duty"].toDouble();
    const double cycles=t*f;
    const double cycle=std::floor(cycles);
    const double threshold=cycles+8*std::numeric_limits<double>::epsilon()*std::abs(cycles);
    if (cycle+duty>threshold) return (cycle+duty)/f;
    if (cycle+1>threshold) return (cycle+1)/f;
    return (cycle+1+duty)/f;
}
}
void simulate(const Circuit& circuit,const Sample& emitSample) {
    circuit.validate();
    auto missing=circuit.missingPins();
    if (!missing.isEmpty()) throw std::runtime_error(("unconnected: "+missing.join(", ")).toStdString());
    const auto mode=circuit.document["simulation"].toObject()["model"].toString("mna");
    if(mode=="pfc-switching" || mode=="pfc-average") {simulatePfc(circuit,emitSample,mode=="pfc-average");return;}
    if(mode!="mna") throw std::runtime_error("unknown simulation model");
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
        if (type=="nmos") d.n=index[pinNet[it.key()+".s"]];
        if (type=="pwm" && (d.params["frequency_Hz"].toDouble()<=0 || d.params["high_V"].toDouble()<=0 ||
                            d.params["duty"].toDouble()<=0 || d.params["duty"].toDouble()>=1))
            throw std::runtime_error("PWM requires positive frequency, high level and duty in (0,1)");
        if (type=="capacitor" || type=="inductor" || type=="dc" || type=="sine" || type=="ammeter"
            || type=="diode" || type=="nmos" || type=="pwm") d.branch=size++;
        else if (type!="resistor" && type!="voltmeter" && type!="sink")
            throw std::runtime_error(("unsupported device: "+d.id).toStdString());
        for (const auto key:{"capacitance_F","inductance_H","resistance_ohm"})
            if (d.params.contains(key) && d.params[key].toDouble()<=0)
                throw std::runtime_error(("non-positive passive value: "+d.id).toStdString());
        d.history=d.params[type=="capacitor" ? "initialVoltage_V" : "initialCurrent_A"].toDouble();
        devices.push_back(d);
    }
    for (auto& d:devices) {
        if (d.type!="nmos") continue;
        for (int i=0;i<static_cast<int>(devices.size());++i)
            if (devices[i].type=="pwm" && devices[i].p==index[pinNet[d.id+".g"]] && devices[i].n==d.n)
                d.gateDriver=i;
        if (d.gateDriver<0) throw std::runtime_error(("missing gate-source PWM driver: "+d.id).toStdString());
    }
    const auto cfg=circuit.document["simulation"].toObject();
    const double step=cfg["timeStep_s"].toDouble(), stop=cfg["stopTime_s"].toDouble();
    const qint64 stride=circuit.outputStride();
    qint64 gridTick=0;
    double t=0,h=0;
    for (;;) {
        // Integrate the interval using its interior switch state; end-point samples
        // report the left limit at discontinuities.
        const double interior=t-h/2;
        bool turnOn=false;
        for(auto& d:devices) {
            if(d.gateDriver<0) continue;
            const bool high=pulseHigh(devices[d.gateDriver],interior);
            if(high && !d.driven) turnOn=true;
            d.driven=high;
        }
        Vector x;
        for (unsigned iteration=0;;++iteration) {
            if (iteration>32) throw std::runtime_error("ideal switch states did not converge");
            System sys(size);
            for (const auto& d:devices) {
                if (d.type=="voltmeter") continue;
                if (d.type=="sink") {
                    const auto amps=d.params["current_A"].toDouble();
                    sys.rhs(d.p,-amps);sys.rhs(d.n,amps);continue;
                }
                if (d.type=="resistor") {
                    const double g=1/d.params["resistance_ohm"].toDouble();
                    sys.add(d.p,d.p,g); sys.add(d.n,d.n,g);
                    sys.add(d.p,d.n,-g); sys.add(d.n,d.p,-g);
                    continue;
                }
                const int k=d.branch;
                sys.add(d.p,k,1); sys.add(d.n,k,-1);
                if (d.type=="diode" || d.type=="nmos") {
                    if (d.driven || d.conducting) {
                        sys.add(k,d.p,1); sys.add(k,d.n,-1);
                    } else sys.add(k,k,1);
                    continue;
                }
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
                else if (d.type=="pwm") sys.rhs(k,pulseHigh(d,interior) ? d.params["high_V"].toDouble() : 0);
                else if (d.type=="sine") sys.rhs(k,d.params["amplitude_V"].toDouble()*
                    std::sin(6.283185307179586*t*d.params["frequency_Hz"].toDouble()));
            }
            try {
                x=sys.solve();
            } catch(const SingularSystem&) {
                // A newly driven MOSFET and the preceding interval's conducting diode
                // can form an impossible voltage-source loop before current is solved.
                // Retry this instant with the diodes blocking, then check their normal
                // forward-voltage/current conditions before accepting any sample.
                if(!turnOn || iteration!=0) throw;
                bool released=false;
                for(auto& d:devices) {
                    if(!d.conducting) continue;
                    d.conducting=false;
                    released=true;
                }
                if(!released) throw;
                continue;
            }
            bool changed=false;
            for (auto& d:devices) {
                if (d.type!="diode" && d.type!="nmos") continue;
                if (d.driven) {
                    d.conducting=false;
                    continue;
                }
                const double sign=d.type=="diode" ? 1 : -1;
                const double forward=sign*(node(x,d.p)-node(x,d.n));
                const bool next=d.conducting ? sign*x[d.branch]>=-1e-12 : forward>1e-12;
                if (next!=d.conducting) { d.conducting=next; changed=true; break; }
            }
            if (!changed) break;
        }
        QMap<QString,double> values;
        for (auto& d:devices) {
            const double v=node(x,d.p)-node(x,d.n);
            if (d.type=="voltmeter") values[d.id+"_V"]=v;
            if (d.type=="ammeter") values[d.id+"_A"]=x[d.branch];
            if (d.type=="capacitor") d.history=v;
            if (d.type=="inductor") d.history=x[d.branch];
            if (d.type=="inductor" || d.type=="diode") values[d.id+"_A"]=x[d.branch];
        }
        const bool onGrid=t==gridTick*step;
        if (!stride || (onGrid && gridTick%stride==0) || t==stop) emitSample(t,values);
        if (t==stop) break;
        if (onGrid) ++gridTick;
        const double boundary=std::min(stop,gridTick*step);
        double next=boundary;
        for (const auto& d:devices)
            if (d.type=="pwm") next=std::min(next,nextPulseEdge(d,t));
        // Numerically coincident clocks represent one physical instant.
        constexpr double epsilon=8*std::numeric_limits<double>::epsilon();
        if (boundary-next<=epsilon*boundary) next=boundary;
        if (stop-next<=epsilon*stop) next=stop;
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
void simulateWave(const Circuit& circuit,const QString& filename) {
    std::unique_ptr<WaveWriter> writer;
    std::vector<double> row;
    simulate(circuit,[&](double time,const QMap<QString,double>& values){
        if (!writer) {
            QStringList channels{"time_s"}; channels.append(values.keys());
            writer=std::make_unique<WaveWriter>(filename,channels);
            row.reserve(channels.size());
        }
        row.clear();row.push_back(time);
        for (double value:values) row.push_back(value);
        writer->append(row);
    });
    writer->finish();
}
