#include "wave-spike.h"
#include "waves.h"
#include <QElapsedTimer>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QTextStream>
#include <algorithm>
#include <cmath>
#include <cstdio>
#include <limits>
#include <stdexcept>

namespace {
double number(const QJsonValue value) {
    const double result=value.toDouble(std::numeric_limits<double>::quiet_NaN());
    if (!std::isfinite(result)) throw std::runtime_error("expected a finite number");
    return result;
}

int integer(const QJsonValue value,int lo,int hi) {
    const double result=number(value);
    if (result<lo || result>hi || result!=std::trunc(result))
        throw std::runtime_error("integer outside accepted range");
    return static_cast<int>(result);
}

QJsonObject query(const Waves& waves,const QJsonObject& request) {
    const auto op=request.value("op").toString();
    const double start=waves.at(0,0),end=waves.at(waves.rows-1,0);
    if (op=="open")
        return {{"channels",QJsonArray::fromStringList(waves.channels)},
                {"domain",QJsonArray{start,end}},{"rows",waves.rows}};
    const int last=waves.channels.size()-1;
    if (op=="at") {
        const auto col=integer(request.value("channel"),0,last);
        QJsonArray samples;
        for (const auto row:request.value("rows").toArray()) {
            const auto index=row.toInteger(-1);
            samples.append(QJsonArray{index,waves.at(index,0),waves.at(index,col)});
        }
        return {{"samples",samples}};
    }
    if (op!="viewport") throw std::runtime_error("unknown wave request");
    const double from=number(request.value("from")),to=number(request.value("to"));
    const int pixels=integer(request.value("pixels"),1,4096);
    const auto channels=request.value("channels").toArray();
    if (channels.empty() || channels.size()>8) throw std::runtime_error("expected 1–8 time channels");
    QJsonArray plots;
    for (const auto channel:channels) {
        const int y=integer(channel,1,last);
        QJsonArray points;
        for (const auto row:waves.envelopeRows(from,to,y,pixels))
            points.append(QJsonArray{row,waves.at(row,0),waves.at(row,y)});
        plots.append(points);
    }
    QJsonObject result{{"plots",plots}};
    if (request.contains("xy")) {
        const auto pair=request.value("xy").toArray();
        const int x=integer(pair.at(0),1,last),y=integer(pair.at(1),1,last);
        const auto xr=waves.envelopeRows(start,end,x,pixels*2);
        const auto yr=waves.envelopeRows(start,end,y,pixels*2);
        std::vector<qint64> rows;
        std::set_union(xr.begin(),xr.end(),yr.begin(),yr.end(),std::back_inserter(rows));
        QJsonArray points;
        for (const auto row:rows) points.append(QJsonArray{row,waves.at(row,x),waves.at(row,y)});
        result.insert("xy",points);
    }
    return result;
}
}

int serveWave(const QString& path) {
    QElapsedTimer timer;
    timer.start();
    const Waves waves(path);
    const auto openMs=timer.nsecsElapsed()/1e6;
    QTextStream input(stdin);
    while (!input.atEnd()) {
        const auto line=input.readLine();
        if (line.isNull()) break;
        QJsonObject result;
        timer.restart();
        try {
            const auto request=QJsonDocument::fromJson(line.toUtf8()).object();
            result=query(waves,request);
            if (request.value("op")=="open") result.insert("openMs",openMs);
        } catch (const std::exception& error) {
            result={{"error",QString::fromUtf8(error.what())}};
        }
        const auto queryMs=timer.nsecsElapsed()/1e6;
        timer.restart();
        const auto body=QJsonDocument(result).toJson(QJsonDocument::Compact);
        const auto encodeMs=timer.nsecsElapsed()/1e6;
        std::fprintf(stdout,"%.6f\t%.6f\t",queryMs,encodeMs);
        std::fwrite(body.constData(),1,body.size(),stdout);
        std::fputc('\n',stdout);
        std::fflush(stdout);
    }
    return 0;
}
