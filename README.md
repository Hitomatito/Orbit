 # Orbit — Gestor de Huellas de Aplicaciones para Linux

 Versión: 0.1.0-alpha

 ## Elevator pitch

 Orbit muestra exactamente qué archivos, servicios y configuraciones deja cada aplicación en tu sistema Linux, y facilita desinstalaciones profundas, auditorías de privacidad y la gestión segura de dependencias.

 ## Problema que resolvemos

 Cuando instalas o desinstalas aplicaciones en Linux no tienes una visión única de "qué dejó atrás" cada aplicación: configuraciones en el home, caches, servicios de systemd, dependencias compartidas y archivos instalados por el gestor de paquetes. Esto provoca acumulación de datos, dificultades para auditar cambios en el sistema y riesgo de romper dependencias al desinstalar paquetes.

 ## Solución y propuesta de valor

 Orbit crea una huella canónica por aplicación (`AppFootprint`) que unifica datos de APT/RPM, Flatpak, Snap y binarios sueltos. Con esa huella podrás:

- Ver y auditar todos los archivos y servicios asociados a una app antes de tomar medidas.
- Generar un plan de desinstalación por etapas con previsualización y backups de configuración.
- Identificar dependencias compartidas y candidatos a huérfanos para evitar eliminar elementos críticos del sistema.
- Evaluar riesgos de privacidad (permisos Flatpak/Snap y apps no sandboxed).

 Todo esto se presenta en una interfaz gráfica clara y en un índice local consultable para búsquedas rápidas.

 ## Características principales (orientadas a valor)

- Descubrimiento unificado de apps: ve todas las apps instaladas sin cambiar de herramienta.
- Huella completa por app: archivos, configs, caches, servicios y procesos en ejecución.
- Desinstalación segura: plan por etapas, backups automáticos y reversibilidad donde sea posible.
- Auditoría de privacidad: puntuación y alertas para permisos sensibles.
- Visualización del sistema: grafo interactivo de dependencias para decisiones informadas.
- Historial de operaciones: registro auditado de cambios y posibilidad de restaurar backups.

 ## ¿A quién va dirigido?

- Usuarios avanzados que quieren mantener sistemas limpios y auditables.
- Administradores que necesitan comprender el impacto de cambios en paquetes.
- Desarrolladores y mantenedores que buscan rastrear huellas de instalaciones.

 ## Quick Start (probar Orbit rápidamente)

1) Instalar la última versión publicada (Flatpak recomendado) o descargar el binario precompilado desde la página de releases.

2) Ejecutar Orbit y permitir que escanee las fuentes del sistema — en minutos tendrás la lista unificada de apps y podrás inspeccionar huellas.

Nota: las operaciones que modifican el sistema solicitarán elevación vía Polkit.

 ## Limitaciones y supuestos

- El descubrimiento de archivos en `$HOME` es heurístico y puede no ser exhaustivo.
- El mapeo de tamaño de dependencias compartidas es aproximado; Orbit muestra desagregaciones y estimaciones.
- Algunas integraciones (p. ej. fanotify en tiempo real) dependen de capacidades del sistema y no siempre están disponibles.

 ## Roadmap (resumen)

- Scaffolding y UI básica
- Adaptadores de paquetes (APT, RPM, Flatpak, Snap)
- Descubrimiento de huellas y mapeo de servicios
- Orquestador de desinstalación con Polkit y backups
- Grafo de dependencias y pulido, empaquetado (Flatpak, .deb, .rpm)

 ## Desarrollo y compilación (para desarrolladores)

 Si quieres compilar desde fuente o contribuir, consulta `DEVELOPMENT.md` para pasos detallados de entorno de desarrollo y pruebas.

 ## Contribuir

- Abre issues para bugs o propuestas de mejora.
- Usa ramas `feature/descripcion` o `fix/descripcion` y abre pull requests con tests y descripción clara.

 ## Licencia

 (Especificar la licencia: MIT/Apache-2.0). Añadir `LICENSE` al repositorio.

 ## Referencias

- Diseño detallado: [ORBIT_Especificacion_Tecnica.md](ORBIT_Especificacion_Tecnica.md)

 ## Contacto

 Proyecto Orbit — Equipo de desarrollo. Abrir issues para comunicaciones.
