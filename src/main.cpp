#include "studio.h"
#include <QApplication>
#include <QDir>
#include <QFileInfo>
#include <QMessageBox>
int main(int argc,char** argv) {
    QApplication app(argc,argv);
    auto args=app.arguments().mid(1);
    const auto bundled=QDir(QCoreApplication::applicationDirPath()).filePath("examples/pfc.litzsim");
    if(args.isEmpty() && QFileInfo::exists(bundled)) args<<bundled;
    Studio studio(args.isEmpty() ? QDir::currentPath() : QFileInfo(args.front()).absolutePath());
    try { for (const auto& file:args) studio.open(file); }
    catch(const std::exception& e) { QMessageBox::critical(nullptr,"LitzSim",e.what()); return 1; }
    studio.show();
    return app.exec();
}
