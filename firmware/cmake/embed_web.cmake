# Generates web_assets.{c,h} (gzip-compressed web interface) in the build dir.
# Sets PSHB_WEB_GEN_DIR and PSHB_WEB_SOURCES for the including project.

find_package(Python3 REQUIRED COMPONENTS Interpreter)

set(PSHB_WEB_DIR ${CMAKE_CURRENT_LIST_DIR}/../src/web)
set(PSHB_WEB_FILES
    ${PSHB_WEB_DIR}/index.html
    ${PSHB_WEB_DIR}/app.js
    ${PSHB_WEB_DIR}/style.css
)
set(PSHB_WEB_GEN_DIR ${CMAKE_BINARY_DIR}/gen/web)
add_custom_command(
    OUTPUT ${PSHB_WEB_GEN_DIR}/web_assets.c ${PSHB_WEB_GEN_DIR}/web_assets.h
    COMMAND Python3::Interpreter ${CMAKE_CURRENT_LIST_DIR}/../tools/embed_assets.py
            ${PSHB_WEB_GEN_DIR} ${PSHB_WEB_FILES}
    DEPENDS ${PSHB_WEB_FILES} ${CMAKE_CURRENT_LIST_DIR}/../tools/embed_assets.py
    COMMENT "Embedding web interface"
)
set(PSHB_WEB_SOURCES ${PSHB_WEB_GEN_DIR}/web_assets.c)
